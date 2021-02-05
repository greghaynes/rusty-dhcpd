use dhcp4r::{options, packet};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use tokio::io::Error as TokioError;
use tokio::net::UdpSocket;
use tokio::sync::Notify;

use crate::config;
use crate::leases;

#[derive(Debug)]
pub enum HandleError {
    IOError(std::io::Error),
    PacketError(),
    NoAvailableLease(),
    FailedSendingReply(),
}

impl std::fmt::Display for HandleError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &*self {
            HandleError::IOError(err) => write!(f, "IOError: {}", err),
            HandleError::PacketError() => write!(f, "Failed parsing packet"),
            HandleError::NoAvailableLease() => write!(f, "No leases available"),
            HandleError::FailedSendingReply() => write!(f, "Failed to send reply packet"),
        }
    }
}

impl From<std::io::Error> for HandleError {
    fn from(error: std::io::Error) -> Self {
        return HandleError::IOError(error);
    }
}

#[derive(Debug)]
pub enum ListenError {
    BindError(TokioError),
    HandleError(HandleError),
}

impl std::fmt::Display for ListenError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &*self {
            ListenError::BindError(tokio_err) => write!(f, "Bind Error: {}", tokio_err),
            ListenError::HandleError(err) => write!(f, "Packet Receive Error: {}", err),
        }
    }
}

pub struct Server {
    config: config::Config,
    lease_block: leases::LeaseBlock,
    stop_notify: Notify,
}

impl Server {
    pub fn create(config: &config::Config) -> Server {
        return Server {
            config: config.clone(),
            lease_block: leases::LeaseBlock::create(
                config.lease_start,
                config.lease_count,
                config.lease_duration,
                config.lease_subnet_mask,
                config.lease_routers.clone(),
                config.lease_domain_servers.clone(),
            ),
            stop_notify: Notify::new(),
        };
    }

    pub async fn serve(&self) -> Result<(), ListenError> {
        match UdpSocket::bind(self.config.bind_address).await {
            Err(err) => return Err(ListenError::BindError(err)),
            Ok(sock) => match self.recv_loop(&sock).await {
                Err(err) => return Err(ListenError::HandleError(err)),
                Ok(()) => return Ok(()),
            },
        }
    }

    pub async fn stop(&self) {
        self.stop_notify.notify_one();
    }

    async fn recv_loop(&self, socket: &tokio::net::UdpSocket) -> Result<(), HandleError> {
        loop {
            let mut buf = [0; 1500];

            tokio::select! {
                _ = socket.readable() => {
                    match socket.try_recv_from(&mut buf) {
                        Ok((n, addr)) => match dhcp4r::packet::Packet::from(&buf[..n]) {
                            Err(_) => {
                                return Err(HandleError::PacketError());
                            }
                            Ok(packet) => return self.handle_packet(socket, &packet, &addr).await,
                        },
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            continue;
                        }
                        Err(e) => {
                            return Err(HandleError::IOError(e));
                        }
                    }
                },
                _ = self.stop_notify.notified() => {
                    return Ok(())
                }
            }
        }
    }

    async fn handle_packet(
        &self,
        socket: &tokio::net::UdpSocket,
        packet: &dhcp4r::packet::Packet,
        src: &std::net::SocketAddr,
    ) -> Result<(), HandleError> {
        match packet.message_type() {
            Ok(options::MessageType::Discover) => {
                // Client requested an address
                if let Some(options::DhcpOption::RequestedIpAddress(addr)) =
                    packet.option(options::REQUESTED_IP_ADDRESS)
                {
                    if self.lease_block.available(&packet.chaddr, addr).await {
                        match self
                            .server_reply(socket, src, options::MessageType::Offer, packet, &addr)
                            .await
                        {
                            Ok(_) => return Ok(()),
                            Err(_) => return Err(HandleError::FailedSendingReply()),
                        }
                    }
                }

                // No address requested or requested address unavailable
                match self.lease_block.get_available(&packet.chaddr).await {
                    Some(address) => {
                        match self
                            .server_reply(
                                socket,
                                src,
                                options::MessageType::Offer,
                                packet,
                                &address,
                            )
                            .await
                        {
                            Ok(_) => return Ok(()),
                            Err(_) => return Err(HandleError::FailedSendingReply()),
                        }
                    }
                    None => return Err(HandleError::NoAvailableLease()),
                }
            }

            Ok(options::MessageType::Request) => {
                let req_ip = match packet.option(options::REQUESTED_IP_ADDRESS) {
                    Some(options::DhcpOption::RequestedIpAddress(x)) => *x,
                    _ => packet.ciaddr,
                };

                if !self.lease_block.available(&packet.chaddr, &req_ip).await {
                    match self
                        .nak(socket, src, packet, "Requested IP not available.")
                        .await
                    {
                        Ok(_) => return Ok(()),
                        Err(_) => return Err(HandleError::FailedSendingReply()),
                    }
                }

                Ok(())
            }

            _ => return Ok(()),
        }
    }

    async fn send(
        &self,
        socket: &tokio::net::UdpSocket,
        src: &std::net::SocketAddr,
        packet: packet::Packet,
    ) -> std::io::Result<usize> {
        let mut out_buff = [0; 1500];
        let mut addr = *src;
        if packet.broadcast || addr.ip() == IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)) {
            addr.set_ip(IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)))
        }
        socket.send_to(packet.encode(&mut out_buff), addr).await
    }

    async fn reply(
        &self,
        socket: &tokio::net::UdpSocket,
        src: &std::net::SocketAddr,
        msg_type: options::MessageType,
        packet: &packet::Packet,
        offer_ip: &Ipv4Addr,
        options: Vec<options::DhcpOption>,
    ) -> std::io::Result<usize> {
        let ciaddr = match msg_type {
            options::MessageType::Nak => Ipv4Addr::new(0, 0, 0, 0),
            _ => packet.ciaddr,
        };

        let mut opts: Vec<options::DhcpOption> = Vec::with_capacity(options.len() + 2);
        opts.push(options::DhcpOption::DhcpMessageType(msg_type));
        opts.push(options::DhcpOption::ServerIdentifier(
            *self.config.bind_address.ip(),
        ));

        self.send(
            socket,
            src,
            packet::Packet {
                reply: true,
                hops: 0,
                xid: packet.xid,
                secs: 0,
                broadcast: packet.broadcast,
                ciaddr: ciaddr,
                yiaddr: *offer_ip,
                siaddr: Ipv4Addr::new(0, 0, 0, 0),
                giaddr: packet.giaddr,
                chaddr: packet.chaddr,
                options: opts,
            },
        )
        .await
    }

    async fn server_reply(
        &self,
        socket: &tokio::net::UdpSocket,
        src: &std::net::SocketAddr,
        msg_type: options::MessageType,
        packet: &packet::Packet,
        offer_ip: &Ipv4Addr,
    ) -> std::io::Result<usize> {
        self.reply(
            socket,
            src,
            msg_type,
            packet,
            offer_ip,
            vec![
                options::DhcpOption::IpAddressLeaseTime(
                    self.lease_block.lease_duration.as_secs() as u32
                ),
                options::DhcpOption::SubnetMask(self.lease_block.subnet_mask),
                options::DhcpOption::Router(self.lease_block.routers.clone()),
                options::DhcpOption::DomainNameServer(self.lease_block.domain_servers.clone()),
            ],
        )
        .await
    }

    async fn nak(
        &self,
        socket: &tokio::net::UdpSocket,
        src: &std::net::SocketAddr,
        req_packet: &packet::Packet,
        message: &str,
    ) -> std::io::Result<usize> {
        self.reply(
            socket,
            src,
            options::MessageType::Nak,
            req_packet,
            &Ipv4Addr::new(0, 0, 0, 0),
            vec![options::DhcpOption::Message(message.to_string())],
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;

    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::time::Duration;

    const SERVER_IP: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
    const SERVER_PORT: u16 = 8998;
    const CLIENT_IP: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 1);
    const CLIENT_PORT: u16 = 8999;
    const CLIENT_HWADDR: &[u8; 6] = b"000000";

    fn discoverPacket() -> dhcp4r::packet::Packet {
        return dhcp4r::packet::Packet {
            reply: false,
            hops: 0,
            xid: 1234,
            secs: 3600,
            broadcast: true,
            ciaddr: CLIENT_IP,
            yiaddr: Ipv4Addr::new(0, 0, 0, 0),
            siaddr: Ipv4Addr::new(0, 0, 0, 0),
            giaddr: Ipv4Addr::new(0, 0, 0, 0),
            chaddr: *CLIENT_HWADDR,
            options: vec![dhcp4r::options::DhcpOption::DhcpMessageType(
                dhcp4r::options::MessageType::Discover,
            )],
        };
    }

    #[tokio::test]
    async fn get_leases() {
        let config = Config {
            bind_address: SocketAddrV4::new(SERVER_IP, SERVER_PORT),
            lease_start: Ipv4Addr::new(10, 41, 0, 0),
            lease_count: 24,
            lease_duration: Duration::from_secs(300),
            lease_subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
            lease_routers: vec![Ipv4Addr::new(10, 41, 1, 1)],
            lease_domain_servers: vec![Ipv4Addr::new(8, 8, 8, 8)],
        };

        let srv = std::sync::Arc::new(super::Server::create(&config));

        // Start server in background
        let srv_bg = srv.clone();
        let srv_handle = tokio::spawn(async move {
            match srv_bg.serve().await {
                Ok(_) => {}
                Err(err) => assert!(false, "Failed to run server: {}", err),
            }
        });

        // Wait for server to start
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // Create client socket
        let client_sock: tokio::net::UdpSocket;
        match tokio::net::UdpSocket::bind(SocketAddrV4::new(CLIENT_IP, CLIENT_PORT)).await {
            Ok(sock) => client_sock = sock,
            Err(err) => {
                assert!(false, "Failed to make client socket: {}", err);
                return ();
            }
        }

        // Send discover
        let buff: &mut [u8; 1500] = &mut [0; 1500];
        discoverPacket().encode(buff);
        match client_sock
            .send_to(buff, std::net::SocketAddrV4::new(SERVER_IP, SERVER_PORT))
            .await
        {
            Ok(_) => println!("Sent discover"),
            Err(err) => assert!(false, "Sending discover: {}", err),
        }

        // Get discover response
        match client_sock.recv_from(buff).await {
            Ok(_) => println!("Got response"),
            Err(err) => assert!(false, "Discover response: {}", err),
        }

        // Start shutdown
        srv.stop().await;
        // Wait until shutdown happens
        match srv_handle.await {
            Ok(_) => assert!(true, "Server success"),
            Err(err) => assert!(false, "Failed when running server: {}", err),
        }
    }
}
