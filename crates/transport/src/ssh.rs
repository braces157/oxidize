//! Native SSH transport support via system `ssh` executable.

use crate::pkt_line::{read_pkt_lines, SidebandDemuxer};
use crate::protocol::{
    build_receive_pack_request, build_upload_pack_request, parse_ref_advertisement, RemoteRef,
    UploadPackDiscovery,
};
use crate::TransportError;
use oxidize_core::id::ObjectId;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Parsed SSH connection endpoint details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshEndpoint {
    /// Host name (or IP address).
    pub host: String,
    /// Optional user name (e.g. `git`).
    pub user: Option<String>,
    /// Optional SSH port (e.g. 22).
    pub port: Option<u16>,
    /// Remote repository path on server.
    pub path: String,
}

/// Checks if a given URL represents an SSH remote.
pub fn is_ssh_url(url: &str) -> bool {
    let trimmed = url.trim();
    if trimmed.starts_with("ssh://") {
        return true;
    }

    // Must not be an HTTP, HTTPS, or FILE URL
    if trimmed.contains("://") {
        return false;
    }

    // Reject Windows drive paths like C:\repo or C:/repo
    if trimmed.len() >= 2
        && trimmed.as_bytes()[0].is_ascii_alphabetic()
        && trimmed.as_bytes()[1] == b':'
    {
        return false;
    }

    // SCP-style format: [user@]host:path
    if let Some(colon_idx) = trimmed.find(':') {
        let before_colon = &trimmed[..colon_idx];
        // Host must not contain slashes
        if !before_colon.contains('/') && !before_colon.contains('\\') && !before_colon.is_empty() {
            return true;
        }
    }

    false
}

/// Parses an SSH remote URL into `SshEndpoint`.
pub fn parse_ssh_url(url: &str) -> Result<SshEndpoint, TransportError> {
    let trimmed = url.trim();

    if let Some(rest) = trimmed.strip_prefix("ssh://") {
        // Form: ssh://[user@]host[:port]/path
        let (auth_host, path_part) = if let Some(slash_pos) = rest.find('/') {
            (&rest[..slash_pos], &rest[slash_pos..])
        } else {
            (rest, "/")
        };

        let (user, host_port) = if let Some(at_pos) = auth_host.find('@') {
            (
                Some(auth_host[..at_pos].to_string()),
                &auth_host[at_pos + 1..],
            )
        } else {
            (None, auth_host)
        };

        let (host, port) = if let Some(colon_pos) = host_port.rfind(':') {
            let p_str = &host_port[colon_pos + 1..];
            if let Ok(p) = p_str.parse::<u16>() {
                (host_port[..colon_pos].to_string(), Some(p))
            } else {
                (host_port.to_string(), None)
            }
        } else {
            (host_port.to_string(), None)
        };

        let path = path_part.trim_start_matches('/').to_string();

        Ok(SshEndpoint {
            host,
            user,
            port,
            path,
        })
    } else if let Some(colon_pos) = trimmed.find(':') {
        // SCP-style: [user@]host:path
        let host_part = &trimmed[..colon_pos];
        let path_part = &trimmed[colon_pos + 1..];

        let (user, host) = if let Some(at_pos) = host_part.find('@') {
            (
                Some(host_part[..at_pos].to_string()),
                host_part[at_pos + 1..].to_string(),
            )
        } else {
            (None, host_part.to_string())
        };

        Ok(SshEndpoint {
            host,
            user,
            port: None,
            path: path_part.to_string(),
        })
    } else {
        Err(TransportError::RefError(format!(
            "invalid SSH URL format: {}",
            url
        )))
    }
}

/// Locates the `ssh` binary on the host system.
pub fn find_ssh_binary() -> Result<PathBuf, TransportError> {
    if let Ok(cmd) = std::env::var("GIT_SSH_COMMAND") {
        if !cmd.trim().is_empty() {
            let first = cmd.split_whitespace().next().unwrap_or("ssh");
            return Ok(PathBuf::from(first));
        }
    }
    if let Ok(cmd) = std::env::var("GIT_SSH") {
        if !cmd.trim().is_empty() {
            return Ok(PathBuf::from(cmd.trim()));
        }
    }

    // Try standard "ssh" in PATH
    if let Ok(status) = Command::new("ssh").arg("-V").output() {
        if status.status.success() || !status.stderr.is_empty() {
            return Ok(PathBuf::from("ssh"));
        }
    }

    // Windows fallback paths
    #[cfg(windows)]
    {
        let git_ssh = Path::new(r"C:\Program Files\Git\usr\bin\ssh.exe");
        if git_ssh.exists() {
            return Ok(git_ssh.to_path_buf());
        }
        let win_ssh = Path::new(r"C:\Windows\System32\OpenSSH\ssh.exe");
        if win_ssh.exists() {
            return Ok(win_ssh.to_path_buf());
        }
    }

    Ok(PathBuf::from("ssh"))
}

/// SSH Git client using the local system's SSH binary.
pub struct SshClient {
    ssh_path: PathBuf,
}

impl Default for SshClient {
    fn default() -> Self {
        Self {
            ssh_path: find_ssh_binary().unwrap_or_else(|_| PathBuf::from("ssh")),
        }
    }
}

impl SshClient {
    /// Creates a new `SshClient`.
    pub fn new() -> Self {
        Self::default()
    }

    fn spawn_ssh_command(
        &self,
        endpoint: &SshEndpoint,
        service: &str,
    ) -> Result<std::process::Child, TransportError> {
        let mut cmd = Command::new(&self.ssh_path);

        if let Some(port) = endpoint.port {
            cmd.arg("-p").arg(port.to_string());
        }

        let destination = if let Some(ref user) = endpoint.user {
            format!("{}@{}", user, endpoint.host)
        } else {
            endpoint.host.clone()
        };
        cmd.arg(destination);

        // Remote service command: e.g. "git-upload-pack 'path/to/repo.git'"
        let remote_cmd = format!("{} '{}'", service, endpoint.path);
        cmd.arg(remote_cmd);

        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::inherit());

        cmd.spawn().map_err(TransportError::Io)
    }

    /// Discovers remote references for `git-upload-pack` over SSH.
    pub fn discover_upload_pack(&self, url: &str) -> Result<UploadPackDiscovery, TransportError> {
        let endpoint = parse_ssh_url(url)?;
        let mut child = self.spawn_ssh_command(&endpoint, "git-upload-pack")?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TransportError::Protocol("failed to capture SSH stdout".to_string()))?;

        let lines = read_pkt_lines(stdout)?;
        let _ = child.kill(); // We only wanted the discovery advertisement for now
        parse_ref_advertisement(&lines)
    }

    /// Fetches a packfile over SSH for the given `wants` and `haves`.
    pub fn fetch_pack(
        &self,
        url: &str,
        wants: &[ObjectId],
        haves: &[ObjectId],
    ) -> Result<(Vec<u8>, Vec<String>), TransportError> {
        let endpoint = parse_ssh_url(url)?;
        let mut child = self.spawn_ssh_command(&endpoint, "git-upload-pack")?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| TransportError::Protocol("failed to capture SSH stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TransportError::Protocol("failed to capture SSH stdout".to_string()))?;

        // Read initial ref advertisement first
        let mut reader = std::io::BufReader::new(stdout);
        let mut initial_lines = Vec::new();
        loop {
            let line = crate::pkt_line::read_single_pkt_line(&mut reader)?;
            let is_flush = line.is_flush();
            initial_lines.push(line);
            if is_flush {
                break;
            }
        }

        // Send wants/haves request
        let body = build_upload_pack_request(wants, haves);
        stdin.write_all(&body)?;
        stdin.flush()?;
        drop(stdin);

        // Read remaining pack lines
        let lines = read_pkt_lines(reader)?;
        let demux = SidebandDemuxer::from_lines(&lines)?;

        if demux.pack_data.is_empty() {
            return Err(TransportError::Protocol(
                "no packfile data received from SSH remote".to_string(),
            ));
        }

        let _ = child.wait();
        Ok((demux.pack_data, demux.progress))
    }

    /// Discovers remote references for `git-receive-pack` over SSH.
    pub fn discover_receive_pack(
        &self,
        url: &str,
    ) -> Result<(Vec<RemoteRef>, Vec<String>), TransportError> {
        let endpoint = parse_ssh_url(url)?;
        let mut child = self.spawn_ssh_command(&endpoint, "git-receive-pack")?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TransportError::Protocol("failed to capture SSH stdout".to_string()))?;

        let lines = read_pkt_lines(stdout)?;
        let (refs, caps, _) = parse_ref_advertisement(&lines)?;
        let _ = child.kill();
        Ok((refs, caps))
    }

    /// Pushes local commits over SSH using `git-receive-pack`.
    pub fn push_pack(
        &self,
        url: &str,
        updates: &[(&ObjectId, &ObjectId, &str)],
        pack_data: &[u8],
    ) -> Result<String, TransportError> {
        let endpoint = parse_ssh_url(url)?;
        let mut child = self.spawn_ssh_command(&endpoint, "git-receive-pack")?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| TransportError::Protocol("failed to capture SSH stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TransportError::Protocol("failed to capture SSH stdout".to_string()))?;

        // Read initial ref advertisement from stdout
        let mut reader = std::io::BufReader::new(stdout);
        loop {
            let line = crate::pkt_line::read_single_pkt_line(&mut reader)?;
            if line.is_flush() {
                break;
            }
        }

        // Send receive-pack request
        let body = build_receive_pack_request(updates, pack_data);
        stdin.write_all(&body)?;
        stdin.flush()?;
        drop(stdin);

        // Read status report
        let lines = read_pkt_lines(reader)?;
        let mut report = Vec::new();
        for line in lines {
            if let Some(text) = line.to_text() {
                report.push(text.to_string());
            }
        }

        let _ = child.wait();
        Ok(report.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_ssh_url() {
        assert!(is_ssh_url("ssh://git@github.com/rust-lang/rust.git"));
        assert!(is_ssh_url("git@github.com:rust-lang/rust.git"));
        assert!(is_ssh_url("user@server.org:repo.git"));
        assert!(!is_ssh_url("https://github.com/rust-lang/rust.git"));
        assert!(!is_ssh_url("http://example.com/repo.git"));
        assert!(!is_ssh_url("file:///c:/repo"));
        assert!(!is_ssh_url("C:\\Users\\PC\\repo"));
        assert!(!is_ssh_url("C:/Users/PC/repo"));
    }

    #[test]
    fn test_parse_ssh_url_scp() {
        let ep = parse_ssh_url("git@github.com:rust-lang/rust.git").unwrap();
        assert_eq!(ep.host, "github.com");
        assert_eq!(ep.user, Some("git".to_string()));
        assert_eq!(ep.port, None);
        assert_eq!(ep.path, "rust-lang/rust.git");
    }

    #[test]
    fn test_parse_ssh_url_protocol() {
        let ep = parse_ssh_url("ssh://git@github.com:2222/rust-lang/rust.git").unwrap();
        assert_eq!(ep.host, "github.com");
        assert_eq!(ep.user, Some("git".to_string()));
        assert_eq!(ep.port, Some(2222));
        assert_eq!(ep.path, "rust-lang/rust.git");
    }
}
