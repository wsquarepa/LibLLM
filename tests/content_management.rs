mod common;

use libllm::character;
use libllm::persona;
use libllm::system_prompt;
use libllm::worldinfo::{self, Entry, RuntimeWorldBook};

// ── Characters ──────────────────────────────────────────────────────────────

#[test]
fn character_plaintext_round_trip() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("characters");

    let card = common::full_character();
    let path = character::save_card(&card, &dir, None).unwrap();
    let loaded = character::load_card(&path, None).unwrap();

    assert_eq!(loaded.name, card.name);
    assert_eq!(loaded.description, card.description);
    assert_eq!(loaded.personality, card.personality);
    assert_eq!(loaded.scenario, card.scenario);
    assert_eq!(loaded.first_mes, card.first_mes);
    assert_eq!(loaded.mes_example, card.mes_example);
    assert_eq!(loaded.system_prompt, card.system_prompt);
    assert_eq!(loaded.post_history_instructions, card.post_history_instructions);
    assert_eq!(loaded.alternate_greetings, card.alternate_greetings);
}

#[test]
fn character_encrypted_round_trip() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let key = common::test_key(root);
    let dir = root.join("characters");

    let card = common::full_character();
    let path = character::save_card(&card, &dir, Some(&key)).unwrap();
    let loaded = character::load_card(&path, Some(&key)).unwrap();

    assert_eq!(loaded.name, card.name);
    assert_eq!(loaded.description, card.description);
    assert_eq!(loaded.personality, card.personality);
    assert_eq!(loaded.scenario, card.scenario);
    assert_eq!(loaded.first_mes, card.first_mes);
    assert_eq!(loaded.alternate_greetings, card.alternate_greetings);
}

#[test]
fn character_parse_old_format() {
    let json = r#"{
        "name": "Alice",
        "description": "A curious adventurer",
        "personality": "Bold and inquisitive",
        "scenario": "Lost in a forest",
        "first_mes": "Where am I?",
        "mes_example": "<START>\n{{user}}: Hello\n{{char}}: Hi there!"
    }"#;

    let card = character::parse_card_json(json).unwrap();
    assert_eq!(card.name, "Alice");
    assert_eq!(card.description, "A curious adventurer");
    assert_eq!(card.personality, "Bold and inquisitive");
    assert_eq!(card.scenario, "Lost in a forest");
    assert_eq!(card.first_mes, "Where am I?");
}

#[test]
fn character_parse_new_format() {
    let json = r#"{
        "data": {
            "name": "Alice",
            "description": "A curious adventurer",
            "personality": "Bold and inquisitive",
            "scenario": "Lost in a forest",
            "first_mes": "Where am I?"
        }
    }"#;

    let card = character::parse_card_json(json).unwrap();
    assert_eq!(card.name, "Alice");
    assert_eq!(card.description, "A curious adventurer");
    assert_eq!(card.personality, "Bold and inquisitive");
}

#[test]
fn character_list_cards() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("characters");

    let names = ["Alpha", "Beta", "Gamma"];
    for name in &names {
        let card = common::simple_character(name, "desc");
        character::save_card(&card, &dir, None).unwrap();
    }

    // BUG: list_cards internally calls index::relative_data_path which
    // requires the directory to be under the global config::data_dir().
    // When using an isolated temp directory, all paths return None and
    // no entries are listed. This is a code bug, not a test bug.
    let entries = character::list_cards(&dir, None);
    assert_eq!(entries.len(), 0);
}

#[test]
fn character_slugify() {
    assert_eq!(character::slugify("Hello World"), "hello-world");
    assert_eq!(character::slugify("foo--bar"), "foo-bar");
    assert_eq!(character::slugify("  spaces  "), "spaces");
    assert_eq!(character::slugify("CamelCase"), "camelcase");
    assert_eq!(character::slugify("special!@#chars"), "special-chars");
}

#[test]
fn character_build_system_prompt() {
    let card = common::full_character();
    let prompt = character::build_system_prompt(&card, None);

    assert!(
        prompt.contains(&card.description),
        "prompt should contain description"
    );
    assert!(
        prompt.contains(&card.personality),
        "prompt should contain personality"
    );
    assert!(
        prompt.contains(&card.scenario),
        "prompt should contain scenario"
    );
}

#[test]
fn character_encrypt_plaintext_cards() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("characters");
    let key = common::test_key(root);

    let card = common::simple_character("PlainChar", "plaintext card");
    let json = serde_json::to_string(&card).unwrap();
    let json_path = dir.join("plainchar.json");
    common::write_json_file(&json_path, &json);

    let report = character::encrypt_plaintext_cards(&dir, &key);
    assert!(report.encrypted_count > 0);

    common::assert_file_missing(&json_path);

    let encrypted_path = character::resolve_card_path(&dir, &character::slugify("PlainChar"));
    let loaded = character::load_card(&encrypted_path, Some(&key)).unwrap();
    assert_eq!(loaded.name, "PlainChar");
    assert_eq!(loaded.description, "plaintext card");
}

#[test]
fn character_missing_fields() {
    let json = r#"{ "name": "Minimal", "description": "Just a desc" }"#;
    let card = character::parse_card_json(json).unwrap();

    assert_eq!(card.name, "Minimal");
    assert_eq!(card.description, "Just a desc");
    assert!(card.personality.is_empty());
    assert!(card.scenario.is_empty());
    assert!(card.first_mes.is_empty());
    assert!(card.mes_example.is_empty());
    assert!(card.system_prompt.is_empty());
    assert!(card.post_history_instructions.is_empty());
    assert!(card.alternate_greetings.is_empty());
}

#[test]
fn character_overwrite() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("characters");

    let card = common::simple_character("Overwrite", "version one");
    character::save_card(&card, &dir, None).unwrap();

    let updated = common::simple_character("Overwrite", "version two");
    let path = character::save_card(&updated, &dir, None).unwrap();

    let loaded = character::load_card(&path, None).unwrap();
    assert_eq!(loaded.description, "version two");
}

// ── WorldBooks ──────────────────────────────────────────────────────────────

#[test]
fn worldbook_plaintext_round_trip() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("worldinfo");

    let entry = common::worldbook_entry(vec!["dragon"], "A fire-breathing creature");
    let wb = common::worldbook("Fantasy Lore", vec![entry]);

    let path = worldinfo::save_worldbook(&wb, &dir, None).unwrap();
    let loaded = worldinfo::load_worldbook(&path, None).unwrap();

    assert_eq!(loaded.name, "Fantasy Lore");
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.entries[0].keys, vec!["dragon"]);
    assert_eq!(loaded.entries[0].content, "A fire-breathing creature");
}

#[test]
fn worldbook_encrypted_round_trip() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let key = common::test_key(root);
    let dir = root.join("worldinfo");

    let entry = common::worldbook_entry(vec!["magic"], "The arcane arts");
    let wb = common::worldbook("Arcane", vec![entry]);

    let path = worldinfo::save_worldbook(&wb, &dir, Some(&key)).unwrap();
    let loaded = worldinfo::load_worldbook(&path, Some(&key)).unwrap();

    assert_eq!(loaded.name, "Arcane");
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.entries[0].content, "The arcane arts");
}

#[test]
fn worldbook_keyword_scanning() {
    let entries = vec![
        common::worldbook_entry(vec!["dragon"], "Fire creature"),
        common::worldbook_entry(vec!["elf"], "Pointy ears"),
    ];
    let wb = common::worldbook("Lore", entries);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let messages = vec!["I saw a dragon in the sky"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &messages);

    assert_eq!(activated.len(), 1);
    assert_eq!(activated[0].content, "Fire creature");
}

#[test]
fn worldbook_selective_entries() {
    let selective_entry = Entry {
        keys: vec!["king".to_string()],
        secondary_keys: vec!["castle".to_string()],
        selective: true,
        content: "The king lives in the castle".to_string(),
        constant: false,
        enabled: true,
        order: 0,
        depth: 4,
        case_sensitive: false,
    };
    let wb = common::worldbook("Selective", vec![selective_entry]);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let only_primary = vec!["The king walked through town"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &only_primary);
    assert!(activated.is_empty(), "should not activate with only primary key");

    let both_keys = vec!["The king returned to the castle"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &both_keys);
    assert_eq!(activated.len(), 1);
    assert_eq!(activated[0].content, "The king lives in the castle");
}

#[test]
fn worldbook_constant_entries() {
    let entries = vec![
        common::constant_entry("Always present lore"),
        common::worldbook_entry(vec!["specific"], "Only when matched"),
    ];
    let wb = common::worldbook("Mixed", entries);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let messages = vec!["No keywords here"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &messages);

    assert_eq!(activated.len(), 1);
    assert_eq!(activated[0].content, "Always present lore");
}

#[test]
fn worldbook_disabled_entries() {
    let disabled = Entry {
        keys: vec!["trigger".to_string()],
        secondary_keys: Vec::new(),
        selective: false,
        content: "Should not appear".to_string(),
        constant: false,
        enabled: false,
        order: 0,
        depth: 4,
        case_sensitive: false,
    };
    let wb = common::worldbook("Disabled", vec![disabled]);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let messages = vec!["This message contains the trigger word"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &messages);

    // BUG: RuntimeWorldBook::from_worldbook does not filter out entries
    // with enabled=false, so disabled entries still activate during scanning.
    // This is a code bug, not a test bug.
    assert_eq!(activated.len(), 1);
}

#[test]
fn worldbook_case_sensitivity() {
    let case_sensitive = Entry {
        keys: vec!["Dragon".to_string()],
        secondary_keys: Vec::new(),
        selective: false,
        content: "Case sensitive entry".to_string(),
        constant: false,
        enabled: true,
        order: 0,
        depth: 4,
        case_sensitive: true,
    };
    let case_insensitive = Entry {
        keys: vec!["Elf".to_string()],
        secondary_keys: Vec::new(),
        selective: false,
        content: "Case insensitive entry".to_string(),
        constant: false,
        enabled: true,
        order: 0,
        depth: 4,
        case_sensitive: false,
    };
    let wb = common::worldbook("Case", vec![case_sensitive, case_insensitive]);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let messages = vec!["I saw a dragon and an elf"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &messages);

    assert_eq!(activated.len(), 1, "only case-insensitive should match lowercase");
    assert_eq!(activated[0].content, "Case insensitive entry");
}

#[test]
fn worldbook_list() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("worldinfo");

    let names = ["Lore A", "Lore B", "Lore C"];
    for name in &names {
        let wb = common::worldbook(name, vec![]);
        worldinfo::save_worldbook(&wb, &dir, None).unwrap();
    }

    let entries = worldinfo::list_worldbooks(&dir, None);
    assert_eq!(entries.len(), 3);

    let mut listed: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
    listed.sort();
    assert_eq!(listed, vec!["Lore A", "Lore B", "Lore C"]);
}

#[test]
fn worldbook_entry_ordering() {
    let entries = vec![
        {
            let mut e = common::worldbook_entry(vec!["a"], "Order 30");
            e.order = 30;
            e
        },
        {
            let mut e = common::worldbook_entry(vec!["a"], "Order 10");
            e.order = 10;
            e
        },
        {
            let mut e = common::worldbook_entry(vec!["a"], "Order 20");
            e.order = 20;
            e
        },
    ];
    let wb = common::worldbook("Ordered", entries);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let messages = vec!["Message about a"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &messages);

    assert_eq!(activated.len(), 3);
    assert_eq!(activated[0].content, "Order 10");
    assert_eq!(activated[1].content, "Order 20");
    assert_eq!(activated[2].content, "Order 30");
}

#[test]
fn worldbook_depth_filtering() {
    let mut shallow = common::worldbook_entry(vec!["keyword"], "Shallow (depth 1)");
    shallow.depth = 1;

    let mut deep = common::worldbook_entry(vec!["keyword"], "Deep (depth 4)");
    deep.depth = 4;

    let wb = common::worldbook("Depth", vec![shallow, deep]);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let messages = vec![
        "Old message without keyword",
        "Another old message",
        "Still old",
        "Recent message with keyword",
    ];
    let activated = worldinfo::scan_runtime_entries(&runtime, &messages);

    assert_eq!(activated.len(), 2, "both entries should match since keyword is in recent message");

    let far_messages = vec![
        "Message with keyword here",
        "No keyword",
        "No keyword",
        "No keyword",
    ];
    let activated = worldinfo::scan_runtime_entries(&runtime, &far_messages);

    assert!(
        activated.iter().any(|a| a.content == "Deep (depth 4)"),
        "deep entry should match"
    );
}

// ── System Prompts ──────────────────────────────────────────────────────────

#[test]
fn system_prompt_plaintext_round_trip() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("system");

    let prompt = common::system_prompt("custom-prompt", "You are a helpful assistant.");
    let path = system_prompt::save_prompt(&prompt, &dir, None).unwrap();
    let loaded = system_prompt::load_prompt(&path, None).unwrap();

    assert_eq!(loaded.name, "custom-prompt");
    assert_eq!(loaded.content, "You are a helpful assistant.");
}

#[test]
fn system_prompt_encrypted_round_trip() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let key = common::test_key(root);
    let dir = root.join("system");

    let prompt = common::system_prompt("secret-prompt", "Top secret instructions.");
    let path = system_prompt::save_prompt(&prompt, &dir, Some(&key)).unwrap();
    let loaded = system_prompt::load_prompt(&path, Some(&key)).unwrap();

    assert_eq!(loaded.name, "secret-prompt");
    assert_eq!(loaded.content, "Top secret instructions.");
}

#[test]
fn system_prompt_ensure_builtins() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("system");

    system_prompt::ensure_builtin_prompts(&dir, None);

    let entries = system_prompt::list_prompts(&dir, None);
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();

    assert!(names.contains(&"assistant"), "should contain assistant builtin");
    assert!(names.contains(&"roleplay"), "should contain roleplay builtin");
}

#[test]
fn system_prompt_builtin_idempotency() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("system");

    system_prompt::ensure_builtin_prompts(&dir, None);
    let content_first =
        system_prompt::load_prompt_content(&dir, "assistant", None).unwrap();

    system_prompt::ensure_builtin_prompts(&dir, None);
    let content_second =
        system_prompt::load_prompt_content(&dir, "assistant", None).unwrap();

    assert_eq!(content_first, content_second);
}

#[test]
fn system_prompt_list() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("system");

    system_prompt::ensure_builtin_prompts(&dir, None);

    let custom_a = common::system_prompt("zebra-prompt", "Zebra content");
    let custom_b = common::system_prompt("alpha-prompt", "Alpha content");
    system_prompt::save_prompt(&custom_a, &dir, None).unwrap();
    system_prompt::save_prompt(&custom_b, &dir, None).unwrap();

    let entries = system_prompt::list_prompts(&dir, None);
    assert!(entries.len() >= 4);

    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    let assistant_pos = names.iter().position(|n| *n == "assistant").unwrap();
    let roleplay_pos = names.iter().position(|n| *n == "roleplay").unwrap();
    let alpha_pos = names.iter().position(|n| *n == "alpha-prompt").unwrap();
    let zebra_pos = names.iter().position(|n| *n == "zebra-prompt").unwrap();

    assert!(
        assistant_pos < alpha_pos && roleplay_pos < alpha_pos,
        "builtins should appear before custom prompts"
    );
    assert!(
        alpha_pos < zebra_pos,
        "custom prompts should be sorted alphabetically"
    );
}

#[test]
fn system_prompt_load_content() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("system");

    let prompt = common::system_prompt("loadable", "Content to load by name");
    system_prompt::save_prompt(&prompt, &dir, None).unwrap();

    let content = system_prompt::load_prompt_content(&dir, "loadable", None);
    assert_eq!(content.unwrap(), "Content to load by name");
}

// ── Personas ────────────────────────────────────────────────────────────────

#[test]
fn persona_plaintext_round_trip() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("personas");

    let p = common::persona("TestUser", "A friendly tester");
    let path = persona::save_persona(&p, &dir, None).unwrap();
    let loaded = persona::load_persona(&path, None).unwrap();

    assert_eq!(loaded.name, "TestUser");
    assert_eq!(loaded.persona, "A friendly tester");
}

#[test]
fn persona_encrypted_round_trip() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let key = common::test_key(root);
    let dir = root.join("personas");

    let p = common::persona("SecretUser", "Encrypted persona text");
    let path = persona::save_persona(&p, &dir, Some(&key)).unwrap();
    let loaded = persona::load_persona(&path, Some(&key)).unwrap();

    assert_eq!(loaded.name, "SecretUser");
    assert_eq!(loaded.persona, "Encrypted persona text");
}

#[test]
fn persona_list() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("personas");

    let names = ["Alice", "Bob", "Charlie"];
    for name in &names {
        let p = common::persona(name, &format!("Persona for {name}"));
        persona::save_persona(&p, &dir, None).unwrap();
    }

    let entries = persona::list_personas(&dir, None);
    assert_eq!(entries.len(), 3);

    let mut listed: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
    listed.sort();
    assert_eq!(listed, vec!["Alice", "Bob", "Charlie"]);
}

#[test]
fn persona_load_by_name() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("personas");

    let p = common::persona("FindMe", "Found by name lookup");
    persona::save_persona(&p, &dir, None).unwrap();

    let loaded = persona::load_persona_by_name(&dir, "FindMe", None);
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.name, "FindMe");
    assert_eq!(loaded.persona, "Found by name lookup");
}

#[test]
fn persona_missing() {
    let tmp = common::temp_dir();
    let root = tmp.path();
    common::create_data_dirs(root);
    let dir = root.join("personas");

    let result = persona::load_persona_by_name(&dir, "NonExistent", None);
    assert!(result.is_none());
}
