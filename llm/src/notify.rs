// Minimal sd_notify, so `Type=notify` units only count as started once the
// default model is loaded and the port is bound.

use std::os::unix::net::{SocketAddr, UnixDatagram};

pub fn ready() {
    let Some(path) = std::env::var_os("NOTIFY_SOCKET") else {
        return;
    };
    let path = path.to_string_lossy().into_owned();
    let addr = match path.strip_prefix('@') {
        Some(name) => {
            use std::os::linux::net::SocketAddrExt;
            SocketAddr::from_abstract_name(name)
        }
        None => SocketAddr::from_pathname(&path),
    };
    let sent = UnixDatagram::unbound()
        .and_then(|sock| addr.and_then(|a| sock.send_to_addr(b"READY=1", &a)));
    if let Err(e) = sent {
        eprintln!("sd_notify failed: {e}");
    }
}
