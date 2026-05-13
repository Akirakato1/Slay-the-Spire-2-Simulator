//! Smoke test: oracle host responds to a ping. Marked #[ignore] until the
//! oracle host has been built (`dotnet build oracle-host -c Release`).

use sts2_sim_oracle_tests::Oracle;

#[test]
#[ignore = "requires `dotnet build oracle-host -c Release`"]
fn oracle_responds_to_ping() {
    let mut oracle = Oracle::spawn().expect("spawn oracle host");
    let response = oracle
        .call("ping", serde_json::json!({}))
        .expect("ping oracle");
    assert_eq!(response["result"], "pong", "unexpected response: {response}");
}
