use rusty_dhcpd::dhcpd::leases::LeaseBlock;
use rusty_dhcpd::dhcpd::server::AbstractServer;
use rusty_dhcpd::web::handlers;
use std::sync::Arc;

struct MockServer<'a> {
    leases: Vec<&'a LeaseBlock>
}

impl AbstractServer for MockServer<'_> {
    fn leases(&self) -> Vec<&LeaseBlock> {
        self.leases()
    }
}

#[test]
fn leases() {
    let dhcpd = MockServer{
        leases: vec![]
    };
    let dhcpd_arc = Arc::new(dhcpd);
    let filter = handlers::filters(dhcpd_arc);

    let res = warp::test::request()
        .path("/leases")
        .reply(&filter);
    assert_eq!(res.status(), 200);
}
