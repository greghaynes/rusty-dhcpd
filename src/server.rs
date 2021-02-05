use dhcp4r::{options, packet};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
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
    logger: slog::Logger,
}

impl Server {
    pub fn create(config: &config::Config, logger: slog::Logger) -> Server {
        let srv_logger = logger.new(
            o!("module" => "server")
        );

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
            logger: srv_logger,
        };
    }

    pub async fn serve(&mut self, shutdown: Arc<Notify>) -> Result<(), ListenError> {
        info!(self.logger, "Serving");
        match UdpSocket::bind(self.config.bind_address).await {
            Err(err) => return Err(ListenError::BindError(err)),
            Ok(sock) => match self.recv_loop(&sock, shutdown).await {
                Err(err) => {
                    error!(self.logger, "Error handling packet: {}", err);
                    return Err(ListenError::HandleError(err));
                },
                Ok(()) => {
                    debug!(self.logger, "Shutting down");
                    return Ok(());
                }
            },
        }
    }

    async fn recv_loop(
        &mut self,
        socket: &tokio::net::UdpSocket,
        shutdown: Arc<Notify>,
    ) -> Result<(), HandleError> {
        let logger = self.logger.new(o!("routine" => "recv_loop"));
        loop {
            let mut buf = [0; 1500];
            debug!(logger, "Waiting for packet");
            tokio::select! {
                _ = socket.readable() => {
                    match socket.try_recv_from(&mut buf) {
                        Ok((n, addr)) => match dhcp4r::packet::Packet::from(&buf[..n]) {
                            Err(e) => {
                                info!(logger, "Failed parsing received packet");
                                return Err(HandleError::PacketError());
                            }
                            Ok(packet) => {
                                debug!(logger, "Got valid DHCP packet");
                                self.handle_packet(socket, &packet, &addr).await?;
                            },
                        },
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            continue;
                        }
                        Err(e) => {
                            warn!(logger, "Error while reading from socket: {}", e);
                            return Err(HandleError::IOError(e));
                        }
                    }
                },
                _ = shutdown.notified() => {
                    info!(logger, "Shutting down");
                    return Ok(())
                }
            }
        }
    }

    async fn handle_packet(
        &mut self,
        socket: &tokio::net::UdpSocket,
        packet: &dhcp4r::packet::Packet,
        src: &std::net::SocketAddr,
    ) -> Result<(), HandleError> {
        let logger = self.logger.new(o!("routine" => "handle_packet"));
        match packet.message_type() {
            // DISCOVER
            Ok(options::MessageType::Discover) => {
                info!(logger, "Got DISCOVER");
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

            // REQUEST
            Ok(options::MessageType::Request) => {
                info!(logger, "Got REQUEST");
                // Use request IP if specified, otherwise client IP
                let req_ip = match packet.option(options::REQUESTED_IP_ADDRESS) {
                    Some(options::DhcpOption::RequestedIpAddress(ip)) => *ip,
                    _ => packet.ciaddr,
                };

                match self.lease_block.reserve(&packet.chaddr, &req_ip).await {
                    Ok(_) => {
                        // We got a lease, send an ACK
                        match self
                            .server_reply(socket, src, options::MessageType::Ack, packet, &req_ip)
                            .await
                        {
                            Ok(_) => return Ok(()),
                            Err(_) => return Err(HandleError::FailedSendingReply()),
                        }
                    }
                    Err(_) => {
                        // Failed to get lease, send a NAK
                        match self
                            .nak(socket, src, packet, "Requested IP not available.")
                            .await
                        {
                            Ok(_) => return Ok(()),
                            Err(_) => return Err(HandleError::FailedSendingReply()),
                        }
                    }
                }
            }

            _ => return {
                warn!(logger, "Received unkonwn packet type");
                Ok(())
            }
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

    use slog::Drain;
    use std::net::{Ipv4Addr, SocketAddrV4};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Notify;

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

    fn requestPacket() -> dhcp4r::packet::Packet {
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
                dhcp4r::options::MessageType::Request,
            )],
        };
    }

    #[tokio::test]
    async fn get_lease() {
        let decorator = slog_term::TermDecorator::new().build();
        let drain = slog_term::FullFormat::new(decorator).build().fuse();
        let drain = slog_async::Async::new(drain).build().fuse();
        let logger = slog::Logger::root(drain, o!());

        let config = Config {
            bind_address: SocketAddrV4::new(SERVER_IP, SERVER_PORT),
            lease_start: Ipv4Addr::new(10, 41, 0, 0),
            lease_count: 24,
            lease_duration: Duration::from_secs(300),
            lease_subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
            lease_routers: vec![Ipv4Addr::new(10, 41, 1, 1)],
            lease_domain_servers: vec![Ipv4Addr::new(8, 8, 8, 8)],
        };

        let shutdown = Arc::new(Notify::new());

        // background shutdown handle
        let shutdown_bg = shutdown.clone();

        let srv_handle = tokio::spawn(async move {
            let mut srv = super::Server::create(&config, logger);
            match srv.serve(shutdown_bg).await {
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
            Ok((n, addr)) => match dhcp4r::packet::Packet::from(&buff[..n]) {
                Err(_) => {
                    assert!(false, "Failed to parse discover response")
                }
                Ok(packet) => {
                    assert_eq!(packet.yiaddr, Ipv4Addr::new(10, 41, 0, 0));
                }
            },
            Err(err) => assert!(false, "Discover response: {}", err),
        }

        // Send request
        let mut req_packet = requestPacket();
        req_packet.options.push(dhcp4r::options::DhcpOption::RequestedIpAddress(Ipv4Addr::new(10, 41, 0, 0)));
        req_packet.encode(buff);
        match client_sock
            .send_to(buff, std::net::SocketAddrV4::new(SERVER_IP, SERVER_PORT))
            .await
        {
            Ok(_) => println!("Sent request"),
            Err(err) => assert!(false, "Sending request: {}", err),
        }

        // Got request response
        match client_sock.recv_from(buff).await {
            Ok((n, addr)) => match dhcp4r::packet::Packet::from(&buff[..n]) {
                Err(_) => {
                    assert!(false, "Failed to parse request response")
                }
                Ok(packet) => {
                    assert_eq!(packet.yiaddr, Ipv4Addr::new(10, 41, 0, 0));
                }
            },
            Err(err) => assert!(false, "Request response: {}", err),
        }

        // Start shutdown
        shutdown.notify_one();
        // Wait until shutdown happens
        match srv_handle.await {
            Ok(_) => assert!(true, "Server success"),
            Err(err) => assert!(false, "Failed when running server: {}", err),
        }
    }
}
