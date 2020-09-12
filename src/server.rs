use dhcp4r::{server, options, packet};

pub struct Server {
}

impl server::Handler for Server {
    fn handle_request(&mut self,
                      server: &server::Server,
                      in_packet: packet::Packet) {
        match in_packet.message_type() {
            Ok(options::MessageType::Discover) => {
                // Handle discover
            }

            Ok(options::MessageType::Request) => {
            }

            Ok(options::MessageType::Release) |
            Ok(options::MessageType::Decline) => {
            }

            _ => {}
        }
    }
}

impl Server {

}
