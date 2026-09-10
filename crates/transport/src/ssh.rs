//! Native SSH transport support via system `ssh` executable.

use crate::pkt_line::{read_pkt_lines, read_pkt_lines_until_flush, SidebandDemuxer};
use crate::protocol::{
    build_receive_pack_request_with_caps, build_upload_pack_request_with_caps,
    parse_ref_advertisement, RemoteRef, UploadPackDiscovery,
};
use crate::TransportError;
use oxidize_core::id::ObjectId;
use std::io::Write;
use std::path::PathBuf;
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

/// Validates an SSH endpoint against option injection and illegal characters.
pub fn validate_ssh_endpoint(endpoint: &SshEndpoint) -> Result<(), TransportError> {
    if endpoint.host.is_empty() {
        return Err(TransportError::RefError(
            "SSH host cannot be empty".to_string(),
        ));
    }
    if endpoint.host.starts_with('-') {
        return Err(TransportError::RefError(format!(
            "invalid SSH host '{}': host cannot start with '-' (option injection prevented)",
            endpoint.host
        )));
    }
    if endpoint
        .host
        .contains(|c: char| c.is_whitespace() || c.is_control())
    {
        return Err(TransportError::RefError(format!(
            "invalid SSH host '{}': host cannot contain whitespace or control characters",
            endpoint.host
        )));
    }
    if let Some(ref user) = endpoint.user {
        if user.is_empty() {
            return Err(TransportError::RefError(
                "SSH username cannot be empty".to_string(),
            ));
        }
        if user.starts_with('-') {
            return Err(TransportError::RefError(format!(
                "invalid SSH username '{}': username cannot start with '-' (option injection prevented)",
                user
            )));
        }
        if user.contains(|c: char| c.is_whitespace() || c.is_control()) {
            return Err(TransportError::RefError(format!(
                "invalid SSH username '{}': username cannot contain whitespace or control characters",
                user
            )));
        }
    }
    if endpoint.path.is_empty() {
        return Err(TransportError::RefError(
            "SSH remote path cannot be empty".to_string(),
        ));
    }
    if endpoint.path.contains('\0') {
        return Err(TransportError::RefError(
            "SSH remote path cannot contain NUL byte".to_string(),
        ));
    }
    Ok(())
}

/// Safely quotes an argument for remote POSIX shell execution by enclosing in single quotes
/// and escaping embedded single quotes as `'\''`.
pub fn sq_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// Finds the SCP delimiter colon outside of any bracketed IPv6 host.
fn find_scp_colon(url: &str) -> Option<usize> {
    let mut in_brackets = false;
    for (idx, b) in url.bytes().enumerate() {
        match b {
            b'[' => in_brackets = true,
            b']' => in_brackets = false,
            b':' if !in_brackets => return Some(idx),
            _ => {}
        }
    }
    None
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
    if let Some(colon_idx) = find_scp_colon(trimmed) {
        let before_colon = &trimmed[..colon_idx];
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

        let (host, port) = if host_port.starts_with('[') {
            if let Some(close_bracket) = host_port.find(']') {
                let h = host_port[1..close_bracket].to_string();
                let remainder = &host_port[close_bracket + 1..];
                let p = if let Some(colon_pos) = remainder.find(':') {
                    remainder[colon_pos + 1..].parse::<u16>().ok()
                } else {
                    None
                };
                (h, p)
            } else {
                return Err(TransportError::RefError(format!(
                    "unclosed bracket in SSH host: {}",
                    host_port
                )));
            }
        } else if let Some(colon_pos) = host_port.rfind(':') {
            let p_str = &host_port[colon_pos + 1..];
            if let Ok(p) = p_str.parse::<u16>() {
                (host_port[..colon_pos].to_string(), Some(p))
            } else {
                (host_port.to_string(), None)
            }
        } else {
            (host_port.to_string(), None)
        };

        // In Git SSH URLs:
        // ssh://host/path -> path is "/path" (absolute)
        // ssh://host/~user/path -> path is "~user/path" (home-relative)
        let path = if let Some(stripped) = path_part.strip_prefix("/~") {
            format!("~{}", stripped)
        } else {
            path_part.to_string()
        };

        let endpoint = SshEndpoint {
            host,
            user,
            port,
            path,
        };
        validate_ssh_endpoint(&endpoint)?;
        Ok(endpoint)
    } else if let Some(colon_pos) = find_scp_colon(trimmed) {
        // SCP-style: [user@]host:path
        let host_part = &trimmed[..colon_pos];
        let path_part = &trimmed[colon_pos + 1..];

        let (user, raw_host) = if let Some(at_pos) = host_part.find('@') {
            (
                Some(host_part[..at_pos].to_string()),
                &host_part[at_pos + 1..],
            )
        } else {
            (None, host_part)
        };

        let host = if raw_host.starts_with('[') && raw_host.ends_with(']') && raw_host.len() >= 2 {
            raw_host[1..raw_host.len() - 1].to_string()
        } else {
            raw_host.to_string()
        };

        let endpoint = SshEndpoint {
            host,
            user,
            port: None,
            path: path_part.to_string(),
        };
        validate_ssh_endpoint(&endpoint)?;
        Ok(endpoint)
    } else {
        Err(TransportError::RefError(format!(
            "invalid SSH URL format: {}",
            url
        )))
    }
}

/// Parses a command line string into tokens, respecting quotes.
/// Windows path backslashes are preserved unless escaping quotes.
pub fn parse_command_tokens(cmd: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = cmd.chars().peekable();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;

    while let Some(c) = chars.next() {
        if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                current.push(c);
            }
        } else if in_double {
            if c == '"' {
                in_double = false;
            } else if c == '\\' {
                if let Some(&next) = chars.peek() {
                    if next == '"' || next == '\\' {
                        chars.next();
                        current.push(next);
                    } else {
                        current.push('\\');
                    }
                } else {
                    current.push('\\');
                }
            } else {
                current.push(c);
            }
        } else {
            match c {
                '\'' => in_single = true,
                '"' => in_double = true,
                '\\' => {
                    if let Some(&next) = chars.peek() {
                        if next.is_whitespace() || next == '"' || next == '\'' {
                            chars.next();
                            current.push(next);
                        } else {
                            current.push('\\');
                        }
                    } else {
                        current.push('\\');
                    }
                }
                c if c.is_whitespace() => {
                    if !current.is_empty() {
                        tokens.push(current);
                        current = String::new();
                    }
                }
                c => current.push(c),
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Describes the SSH binary and command-line arguments to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshInvocation {
    /// Executable program path.
    pub program: PathBuf,
    /// Arguments that precede endpoint and remote command.
    pub extra_args: Vec<String>,
}

impl SshInvocation {
    /// Creates a new `SshInvocation`.
    pub fn new(program: PathBuf, extra_args: Vec<String>) -> Self {
        Self {
            program,
            extra_args,
        }
    }

    /// Detects SSH invocation from `GIT_SSH_COMMAND`, `GIT_SSH`, or system fallback.
    pub fn detect() -> Self {
        if let Ok(cmd) = std::env::var("GIT_SSH_COMMAND") {
            let trimmed = cmd.trim();
            if !trimmed.is_empty() {
                let path_as_is = PathBuf::from(trimmed);
                if path_as_is.is_file() {
                    return Self {
                        program: path_as_is,
                        extra_args: Vec::new(),
                    };
                }
                let tokens = parse_command_tokens(trimmed);
                if !tokens.is_empty() {
                    return Self {
                        program: PathBuf::from(&tokens[0]),
                        extra_args: tokens[1..].to_vec(),
                    };
                }
            }
        }
        if let Ok(cmd) = std::env::var("GIT_SSH") {
            let trimmed = cmd.trim();
            if !trimmed.is_empty() {
                // Per Git documentation, GIT_SSH is the program to run without arguments.
                // The entire string is the executable path, preserving spaces.
                return Self {
                    program: PathBuf::from(trimmed),
                    extra_args: Vec::new(),
                };
            }
        }

        // Try standard "ssh" in PATH
        if let Ok(status) = Command::new("ssh").arg("-V").output() {
            if status.status.success() || !status.stderr.is_empty() {
                return Self {
                    program: PathBuf::from("ssh"),
                    extra_args: Vec::new(),
                };
            }
        }

        // Windows fallback paths
        #[cfg(windows)]
        {
            let git_ssh = PathBuf::from(r"C:\Program Files\Git\usr\bin\ssh.exe");
            if git_ssh.exists() {
                return Self {
                    program: git_ssh,
                    extra_args: Vec::new(),
                };
            }
            let win_ssh = PathBuf::from(r"C:\Windows\System32\OpenSSH\ssh.exe");
            if win_ssh.exists() {
                return Self {
                    program: win_ssh,
                    extra_args: Vec::new(),
                };
            }
        }

        Self {
            program: PathBuf::from("ssh"),
            extra_args: Vec::new(),
        }
    }
}

/// Locates the `ssh` binary on the host system.
pub fn find_ssh_binary() -> Result<PathBuf, TransportError> {
    Ok(SshInvocation::detect().program)
}

/// SSH Git client using the local system's SSH binary.
pub struct SshClient {
    invocation: SshInvocation,
}

impl Default for SshClient {
    fn default() -> Self {
        Self {
            invocation: SshInvocation::detect(),
        }
    }
}

impl SshClient {
    /// Creates a new `SshClient`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an `SshClient` with a specific invocation configuration.
    pub fn with_invocation(invocation: SshInvocation) -> Self {
        Self { invocation }
    }

    /// Returns a reference to the active `SshInvocation`.
    pub fn invocation(&self) -> &SshInvocation {
        &self.invocation
    }

    /// Builds the program path and argv list for an SSH invocation.
    pub fn build_command_args(
        &self,
        endpoint: &SshEndpoint,
        service: &str,
    ) -> Result<(PathBuf, Vec<String>), TransportError> {
        validate_ssh_endpoint(endpoint)?;

        let mut args = self.invocation.extra_args.clone();
        if let Some(port) = endpoint.port {
            args.push("-p".to_string());
            args.push(port.to_string());
        }

        let destination = if let Some(ref user) = endpoint.user {
            format!("{}@{}", user, endpoint.host)
        } else {
            endpoint.host.clone()
        };
        args.push(destination);

        let remote_cmd = format!("{} {}", service, sq_quote(&endpoint.path));
        args.push(remote_cmd);

        Ok((self.invocation.program.clone(), args))
    }

    fn spawn_ssh_command(
        &self,
        endpoint: &SshEndpoint,
        service: &str,
    ) -> Result<std::process::Child, TransportError> {
        let (program, args) = self.build_command_args(endpoint, service)?;
        let mut cmd = Command::new(program);
        cmd.args(args);
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

        let lines = read_pkt_lines_until_flush(stdout)?;
        let _ = child.kill(); // We only wanted the discovery advertisement for now
        let _ = child.wait();
        parse_ref_advertisement(&lines)
    }

    /// Fetches a packfile over SSH for the given `wants` and `haves`.
    pub fn fetch_pack(
        &self,
        url: &str,
        wants: &[ObjectId],
        haves: &[ObjectId],
    ) -> Result<(Vec<u8>, Vec<String>), TransportError> {
        self.fetch_pack_with_caps(url, wants, haves, &[])
    }

    /// Fetches a packfile over SSH using negotiated server capabilities.
    pub fn fetch_pack_with_caps(
        &self,
        url: &str,
        wants: &[ObjectId],
        haves: &[ObjectId],
        server_caps: &[String],
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
        let _initial_lines = read_pkt_lines_until_flush(&mut reader)?;

        // Send wants/haves request
        let body = build_upload_pack_request_with_caps(wants, haves, server_caps);
        stdin.write_all(&body)?;
        stdin.flush()?;
        drop(stdin);

        // Read streaming sideband pack lines
        let demux = SidebandDemuxer::read_stream(&mut reader)?;

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

        let lines = read_pkt_lines_until_flush(stdout)?;
        let (refs, caps, _) = parse_ref_advertisement(&lines)?;
        let _ = child.kill();
        let _ = child.wait();
        Ok((refs, caps))
    }

    /// Pushes local commits over SSH using `git-receive-pack`.
    pub fn push_pack(
        &self,
        url: &str,
        updates: &[(&ObjectId, &ObjectId, &str)],
        pack_data: &[u8],
    ) -> Result<crate::protocol::PushReport, TransportError> {
        self.push_pack_with_caps(url, updates, pack_data, &[])
    }

    /// Pushes local commits over SSH using `git-receive-pack` and negotiated capabilities.
    pub fn push_pack_with_caps(
        &self,
        url: &str,
        updates: &[(&ObjectId, &ObjectId, &str)],
        pack_data: &[u8],
        server_caps: &[String],
    ) -> Result<crate::protocol::PushReport, TransportError> {
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
        let _initial_lines = read_pkt_lines_until_flush(&mut reader)?;

        // Send receive-pack request
        let body = build_receive_pack_request_with_caps(updates, pack_data, server_caps);
        stdin.write_all(&body)?;
        stdin.flush()?;
        drop(stdin);

        // Read status report
        let lines = read_pkt_lines(reader)?;
        let report = crate::protocol::parse_push_report(&lines)?;

        let exit_status = child.wait().map_err(TransportError::Io)?;
        if !exit_status.success() && report.is_success() {
            return Err(TransportError::Protocol(format!(
                "git-receive-pack process failed with exit status {}",
                exit_status
            )));
        }

        Ok(report)
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
        assert!(is_ssh_url("git@[::1]:repo.git"));
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

        let ep_abs = parse_ssh_url("git@github.com:/var/git/rust.git").unwrap();
        assert_eq!(ep_abs.path, "/var/git/rust.git");
    }

    #[test]
    fn test_parse_ssh_url_protocol_preserves_absolute_path() {
        let ep = parse_ssh_url("ssh://git@example.invalid/absolute/repo.git").unwrap();
        assert_eq!(ep.host, "example.invalid");
        assert_eq!(ep.user, Some("git".to_string()));
        assert_eq!(ep.port, None);
        assert_eq!(ep.path, "/absolute/repo.git");

        let ep_port = parse_ssh_url("ssh://git@github.com:2222/rust-lang/rust.git").unwrap();
        assert_eq!(ep_port.host, "github.com");
        assert_eq!(ep_port.user, Some("git".to_string()));
        assert_eq!(ep_port.port, Some(2222));
        assert_eq!(ep_port.path, "/rust-lang/rust.git");

        let ep_home = parse_ssh_url("ssh://git@example.invalid/~user/repo.git").unwrap();
        assert_eq!(ep_home.path, "~user/repo.git");
    }

    #[test]
    fn test_parse_ssh_url_ipv6() {
        let ep = parse_ssh_url("ssh://[::1]:2222/repo.git").unwrap();
        assert_eq!(ep.host, "::1");
        assert_eq!(ep.port, Some(2222));
        assert_eq!(ep.path, "/repo.git");

        let ep_scp = parse_ssh_url("git@[::1]:repo.git").unwrap();
        assert_eq!(ep_scp.host, "::1");
        assert_eq!(ep_scp.user, Some("git".to_string()));
        assert_eq!(ep_scp.path, "repo.git");
    }

    #[test]
    fn test_option_shaped_destinations_rejected() {
        assert!(parse_ssh_url("ssh://-oProxyCommand=calc.exe/repo.git").is_err());
        assert!(parse_ssh_url("-oProxyCommand=calc.exe:repo.git").is_err());
        assert!(parse_ssh_url("ssh://-v@host/repo.git").is_err());
        assert!(parse_ssh_url("-v@host:repo.git").is_err());
    }

    #[test]
    fn test_sq_quote() {
        assert_eq!(sq_quote("simple"), "'simple'");
        assert_eq!(sq_quote("with space"), "'with space'");
        assert_eq!(sq_quote("with'quote"), "'with'\\''quote'");
        assert_eq!(sq_quote("cmd; rm -rf /"), "'cmd; rm -rf /'");
    }

    #[test]
    fn test_parse_command_tokens() {
        let tokens =
            parse_command_tokens(r#""C:\Program Files\OpenSSH\ssh.exe" -v -o "BatchMode=yes""#);
        assert_eq!(
            tokens,
            vec![
                r"C:\Program Files\OpenSSH\ssh.exe",
                "-v",
                "-o",
                "BatchMode=yes"
            ]
        );

        let simple = parse_command_tokens("ssh -v -i /key/path");
        assert_eq!(simple, vec!["ssh", "-v", "-i", "/key/path"]);
    }
}
