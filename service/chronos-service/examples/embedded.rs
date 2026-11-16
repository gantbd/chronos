use chronos_engine::{Engine, StateRevision, TaskSpecV2};
use chronos_host::ClientV1;
use chronos_host::native::{RequestV1, ResponseV1};
use chronos_service::{Authorizer, Limits, Service};

// This executable is a trusted local composition root, not a credential check.
struct LocalOperator;
impl Authorizer for LocalOperator {
    fn allows(&mut self, _: &RequestV1) -> bool {
        true
    }
}

fn main() {
    let mut client = Service::new(
        Engine::new(),
        LocalOperator,
        Limits {
            max_graph_tasks: 16,
            max_page_entries: 16,
        },
    );
    let reply = client
        .call(RequestV1::SubmitGraph {
            expected: StateRevision::ZERO,
            tasks: vec![
                TaskSpecV2::new("prepare"),
                TaskSpecV2::new("review").depends_on("prepare"),
            ],
        })
        .unwrap()
        .unwrap();
    assert_eq!(reply, ResponseV1::SubmitGraph(Ok(())));
    let reply = client
        .call(RequestV1::Page {
            cursor: None,
            limit: 16,
        })
        .unwrap()
        .unwrap();
    let ResponseV1::Page(Ok(page)) = reply else {
        panic!("expected page")
    };
    assert_eq!(page.entries.len(), 2);
    println!(
        "Admitted and observed {} tasks through the native client.",
        page.entries.len()
    );
}
