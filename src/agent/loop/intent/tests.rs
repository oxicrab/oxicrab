use super::*;

#[test]
fn test_action_intent_positive() {
    let cases = [
        "Create a task to feed the cat at 9pm",
        "Add a reminder for tomorrow",
        "Delete the old config file",
        "Please schedule a job for 4pm",
        "Can you send an email to the team?",
        "Show me my tasks",
        "List all open issues",
        "Close that task",
        "Complete the first one",
        "Mark it done",
        "Search for the latest report",
        "Run the deployment script",
        "Update the settings",
        "Can you check the status?",
        "Just create it",
        "Go ahead and delete it",
        "I need you to send the form",
        "Could you find the document?",
        "Would you remove the old entries?",
        "Save the changes",
        "open the file",
        "move that to the archive",
        "pls add a task",
        "don't forget to send the report",
    ];
    for text in cases {
        assert!(classify_action_intent(text), "should be action: {}", text);
    }
}

#[test]
fn test_action_intent_negative() {
    let cases = [
        "How are you?",
        "Thanks!",
        "Good morning",
        "ok",
        "Tell me about creating tasks",
        "How do I delete a file?",
        "How to schedule a cron job",
        "Explain how the search works",
        "What is a task?",
        "What does the delete action do?",
        "What if I schedule it for later?",
        "What happens when you close a task?",
        "Describe the update process",
        "Why does the build fail?",
        "Don't create anything yet",
        "Do not delete that",
        "Never remove the config",
        "When should I run the migration?",
        "Where is the config file?",
        "hi",
        "yes",
        "no",
        "",
    ];
    for text in cases {
        assert!(
            !classify_action_intent(text),
            "should NOT be action: {}",
            text
        );
    }
}

#[test]
fn test_clarification_question_positive() {
    let cases = [
        "Which task would you like me to close?",
        "What should the task name be?",
        "Could you specify which file?",
        "Do you want me to delete all of them?",
        "Did you mean the first or second one?",
        "Sure, but which one?",
        "What's the due date?",
    ];
    for text in cases {
        assert!(
            is_clarification_question(text),
            "should be clarification: {}",
            text
        );
    }
}

#[test]
fn test_clarification_question_negative() {
    let cases = [
        "Created: Feed the cat — due today at 9pm.",
        "Both created:\n• Task A\n• Task B",
        "Done! All set.",
        "I've scheduled the job for 4pm.",
        // Long responses with ? aren't simple clarification
        &format!("{}?", "a".repeat(250)),
        // Action claims with trailing ? should NOT escape as clarification
        "I've completed the task, should I do anything else?",
        "I've updated the config. Need anything else?",
        "I've saved the changes. Want me to continue?",
    ];
    for text in cases {
        assert!(
            !is_clarification_question(text),
            "should NOT be clarification: {}",
            text
        );
    }
}

#[test]
fn test_action_prototypes_are_valid() {
    // Ensure prototypes list is non-empty and has no duplicates
    assert!(!ACTION_PROTOTYPES.is_empty());
    let mut seen = std::collections::HashSet::new();
    for proto in ACTION_PROTOTYPES {
        assert!(seen.insert(*proto), "duplicate action prototype: {}", proto);
        assert!(proto.len() >= 5, "prototype too short: {}", proto);
    }
}

#[test]
fn test_semantic_threshold_is_reasonable() {
    // Threshold should be in a reasonable range for BGE-small-en-v1.5
    const {
        assert!(SEMANTIC_ACTION_THRESHOLD > 0.5);
        assert!(SEMANTIC_ACTION_THRESHOLD < 0.9);
    }
}
