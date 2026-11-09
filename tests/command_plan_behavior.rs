use chronos_engine::{
    BoundedWord, CommandKindV1, CommandOutcomeV1, CommandV1, Engine, PlanError, Task, TaskId,
    TaskState,
};

#[test]
fn bounded_command_words_interpret_in_order_through_the_authoritative_engine() {
    let mut engine = Engine::new();
    let plan = BoundedWord::<_, 4>::from_items([
        CommandV1::Submit(Task::new("task", "work")),
        CommandV1::Cancel(TaskId::from("task")),
    ]);

    let outcomes = engine.execute_plan(plan).unwrap();

    assert_eq!(
        outcomes,
        [
            CommandOutcomeV1::Submit(Ok(())),
            CommandOutcomeV1::Cancel(Ok(())),
        ]
    );
    assert!(matches!(
        engine.task(&TaskId::from("task")).unwrap().state,
        TaskState::Cancelled
    ));
}

#[test]
fn interpretation_respects_word_composition_with_state_threading() {
    let left = BoundedWord::<_, 4>::singleton(CommandV1::Submit(Task::new("a", "one")));
    let right = BoundedWord::<_, 4>::from_items([
        CommandV1::Submit(Task::new("b", "two")),
        CommandV1::Cancel(TaskId::from("a")),
    ]);
    let composed = BoundedWord::<_, 4>::singleton(CommandV1::Submit(Task::new("a", "one"))).concat(
        BoundedWord::<_, 4>::from_items([
            CommandV1::Submit(Task::new("b", "two")),
            CommandV1::Cancel(TaskId::from("a")),
        ]),
    );
    let mut folded = Engine::new();
    let mut split = Engine::new();

    let folded_outcomes = folded.execute_plan(composed).unwrap();
    let mut split_outcomes = split.execute_plan(left).unwrap();
    split_outcomes.extend(split.execute_plan(right).unwrap());

    assert_eq!(folded_outcomes, split_outcomes);
    for id in ["a", "b"] {
        assert_eq!(
            folded.task(&TaskId::from(id)),
            split.task(&TaskId::from(id))
        );
    }
}

#[test]
fn non_executable_words_refuse_before_any_transition_and_domain_refusals_continue() {
    let mut engine = Engine::new();
    assert_eq!(
        engine.execute_plan(BoundedWord::<CommandV1, 1>::Overflow),
        Err(PlanError::Overflow)
    );
    assert_eq!(engine.tasks().count(), 0);

    let plan = BoundedWord::<_, 3>::from_items([
        CommandV1::Cancel(TaskId::from("missing")),
        CommandV1::Submit(Task::new("task", "work")),
    ]);
    let outcomes = engine.execute_plan(plan).unwrap();
    assert!(matches!(outcomes[0], CommandOutcomeV1::Cancel(Err(_))));
    assert_eq!(outcomes[1], CommandOutcomeV1::Submit(Ok(())));
}

#[test]
fn command_schema_is_versioned_closed_and_debug_redacted() {
    let command = CommandV1::Submit(Task::new("sensitive-id", "sensitive summary"));
    assert_eq!(command.kind(), CommandKindV1::Submit);
    let debug = format!("{command:?}");
    assert!(!debug.contains("sensitive-id"));
    assert!(!debug.contains("sensitive summary"));
}
