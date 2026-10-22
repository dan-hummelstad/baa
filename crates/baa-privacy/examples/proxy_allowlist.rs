// Consumed by the §5.3 CI proxy test (mitmproxy addon reads one host per line).
fn main() {
    for host in baa_privacy::PROXY_ALLOWLIST {
        println!("{host}");
    }
}
