//! Keep media on the local address authenticated by the control connection.
//! Wildcard listeners can accept connections through different interfaces.
use axum::{Extension, Router};
use axum_server::accept::Accept;
use std::{
    future::{Ready, ready},
    io,
    net::{SocketAddr, UdpSocket},
};
use tokio::net::TcpStream;

#[derive(Clone, Copy)]
pub(crate) struct Connection {
    pub remote: SocketAddr,
    pub local: SocketAddr,
}
impl Connection {
    pub fn media_socket(self, port: u16) -> io::Result<UdpSocket> {
        let mut local = self.local;
        local.set_port(0); // Retain the IPv6 scope ID of the actual TCP endpoint.
        let socket = UdpSocket::bind(local)?;
        let mut remote = self.remote;
        remote.set_port(port);
        socket.connect(remote)?;
        socket.set_nonblocking(true)?;
        Ok(socket)
    }
}

#[derive(Clone)]
pub(crate) struct AddressAcceptor;
impl Accept<TcpStream, Router> for AddressAcceptor {
    type Stream = TcpStream;
    type Service = Router;
    type Future = Ready<io::Result<(TcpStream, Router)>>;
    fn accept(&self, stream: TcpStream, service: Router) -> Self::Future {
        ready((|| {
            let connection = Connection {
                remote: stream.peer_addr()?,
                local: stream.local_addr()?,
            };
            Ok((stream, service.layer(Extension(connection))))
        })())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn media_uses_control_local_address_instead_of_default_route() {
        let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
        peer.set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        let connection = Connection {
            remote: peer.local_addr().unwrap(),
            local: "127.0.0.1:5001".parse().unwrap(),
        };
        let socket = connection
            .media_socket(peer.local_addr().unwrap().port())
            .unwrap();
        socket.send(b"media-path").unwrap();
        let mut bytes = [0; 32];
        let (count, source) = peer.recv_from(&mut bytes).unwrap();
        assert_eq!(&bytes[..count], b"media-path");
        assert_eq!(source.ip(), connection.local.ip());
    }
    #[test]
    fn unavailable_control_address_does_not_fall_back_to_another_interface() {
        let connection = Connection {
            remote: "127.0.0.1:5000".parse().unwrap(),
            local: "192.0.2.1:5001".parse().unwrap(),
        };
        assert_eq!(
            connection.media_socket(5000).unwrap_err().kind(),
            io::ErrorKind::AddrNotAvailable
        );
    }
}
