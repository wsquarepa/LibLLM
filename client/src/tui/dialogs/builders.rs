use super::FieldDialog;
use super::validation::FieldValidation;

pub(in crate::tui) const DIALOG_WIDTH_RATIO: f32 = 0.7;
pub(in crate::tui) const DIALOG_HEIGHT_RATIO: f32 = 0.6;
pub(in crate::tui) const LIST_DIALOG_WIDTH: u16 = 50;
pub(in crate::tui) const LIST_DIALOG_TALL_PADDING: u16 = 4;
pub(in crate::tui) const FIELD_DIALOG_DEFAULT_WIDTH: u16 = 60;

const CONFIG_FIELDS: &[&str] = &[
    "API URL",
    "",
    "Template",
    "Instruct",
    "Reasoning",
    "",
    "Temperature",
    "Top-K",
    "Top-P",
    "Min-P",
    "Repeat Last N",
    "Repeat Penalty",
    "Max Tokens",
    "TLS Skip Verify",
    "Debug Logging",
];
const CONFIG_BOOLEAN_FIELDS: &[usize] = &[13, 14];
const CONFIG_SEPARATOR_FIELDS: &[usize] = &[1, 5];
const CONFIG_SELECTOR_FIELDS: &[usize] = &[2, 3, 4];

const TEMPLATE_EDITOR_FIELDS: &[&str] =
    &["Name", "Story String", "Example Separator", "Chat Start"];
const TEMPLATE_EDITOR_MULTILINE: &[usize] = &[1];

const INSTRUCT_EDITOR_FIELDS: &[&str] = &[
    "Name",
    "Input Sequence",
    "Output Sequence",
    "System Sequence",
    "Input Suffix",
    "Output Suffix",
    "System Suffix",
    "Stop Sequence",
    "Separator Sequence",
    "Wrap",
    "System Same As User",
    "Seq. As Stop Strings",
];
const INSTRUCT_EDITOR_BOOLEAN: &[usize] = &[9, 10, 11];

const REASONING_EDITOR_FIELDS: &[&str] = &["Name", "Prefix", "Suffix", "Separator"];

const PERSONA_FIELDS: &[&str] = &["Name", "Persona"];
const PERSONA_MULTILINE: &[usize] = &[1];

const CHARACTER_EDITOR_FIELDS: &[&str] = &[
    "Name",
    "Description",
    "Personality",
    "Scenario",
    "First Message",
    "Examples",
    "System Prompt",
    "Post-History",
];
const CHARACTER_EDITOR_MULTILINE: &[usize] = &[1, 2, 3, 4, 5, 6, 7];

const SYSTEM_PROMPT_FIELDS: &[&str] = &["Name", "Content"];
const SYSTEM_PROMPT_MULTILINE: &[usize] = &[1];

const ENTRY_EDITOR_FIELDS: &[&str] = &[
    "Keys [OR]",
    "Content",
    "Selective",
    "Keys [AND]",
    "Constant",
    "Enabled",
    "Order",
    "Depth",
    "Case Sensitive",
];
const ENTRY_EDITOR_MULTILINE: &[usize] = &[1];
const ENTRY_EDITOR_PLACEHOLDER_FIELDS: &[usize] = &[0, 3];

pub fn open_config_editor(values: Vec<String>, locked_fields: Vec<usize>) -> FieldDialog<'static> {
    FieldDialog::new(" Configuration ", CONFIG_FIELDS, values, &[])
        .with_boolean_fields(CONFIG_BOOLEAN_FIELDS)
        .with_locked_fields(locked_fields)
        .with_separator_fields(CONFIG_SEPARATOR_FIELDS)
        .with_selector_fields(CONFIG_SELECTOR_FIELDS)
        .with_validated_fields(vec![
            (6, FieldValidation::Float { min: 0.0, max: 2.0 }),
            (7, FieldValidation::Int { min: 1, max: 100 }),
            (8, FieldValidation::Float { min: 0.0, max: 1.0 }),
            (9, FieldValidation::Float { min: 0.0, max: 1.0 }),
            (
                10,
                FieldValidation::Int {
                    min: -1,
                    max: 32767,
                },
            ),
            (11, FieldValidation::Float { min: 0.0, max: 2.0 }),
            (
                12,
                FieldValidation::Int {
                    min: -1,
                    max: 32767,
                },
            ),
        ])
}

pub fn open_persona_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(" Edit Persona ", PERSONA_FIELDS, values, PERSONA_MULTILINE)
        .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_character_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Character ",
        CHARACTER_EDITOR_FIELDS,
        values,
        CHARACTER_EDITOR_MULTILINE,
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_template_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Template Preset ",
        TEMPLATE_EDITOR_FIELDS,
        values,
        TEMPLATE_EDITOR_MULTILINE,
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_instruct_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Instruct Preset ",
        INSTRUCT_EDITOR_FIELDS,
        values,
        &[],
    )
    .with_boolean_fields(INSTRUCT_EDITOR_BOOLEAN)
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_reasoning_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Reasoning Preset ",
        REASONING_EDITOR_FIELDS,
        values,
        &[],
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_system_prompt_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit System Prompt ",
        SYSTEM_PROMPT_FIELDS,
        values,
        SYSTEM_PROMPT_MULTILINE,
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_entry_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Entry ",
        ENTRY_EDITOR_FIELDS,
        values,
        ENTRY_EDITOR_MULTILINE,
    )
    .with_placeholder("keyword1, keyword2, ...", ENTRY_EDITOR_PLACEHOLDER_FIELDS)
    .with_validated_fields(vec![
        (
            6,
            FieldValidation::Int {
                min: -999,
                max: 999,
            },
        ),
        (7, FieldValidation::Int { min: 0, max: 24 }),
    ])
}

pub fn open_entry_editor_non_selective(values: Vec<String>) -> FieldDialog<'static> {
    let mut dialog = open_entry_editor(values);
    dialog.hidden_fields = vec![3];
    dialog
}
