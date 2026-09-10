use oxidize_pack::delta::apply_delta;
use oxidize_transport::pkt_line::{read_pkt_lines, PktLine, SidebandDemuxer};
use oxidize_transport::ssh::parse_ssh_url;

fn main() {
    let result = std::panic::catch_unwind(|| apply_delta(b"a", &[1, 1, 0x91]));
    println!("truncated_delta_panics={}", result.is_err());
    let frames = vec![PktLine::Data(b"NAK\n".to_vec()), PktLine::Data(b"\x01PACK".to_vec())];
    let demux = SidebandDemuxer::from_lines(&frames).expect("probe demux");
    println!("demux_pack_bytes={:?}", demux.pack_data);
    println!("truncated_prefix_accepted={}", read_pkt_lines(&b"00"[..]).is_ok());
    println!("ssh_absolute_path={:?}", parse_ssh_url("ssh://git@example.invalid/absolute/repo.git").unwrap().path);
}
