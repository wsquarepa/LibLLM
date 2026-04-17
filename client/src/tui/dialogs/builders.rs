//! Factory functions for constructing field editor dialogs with validation rules.

use super::FieldDialog;
use super::tabbed_field::{TabSection, TabbedFieldDialog};
use super::validation::FieldValidation;

pub(in crate::tui) const DIALOG_WIDTH_RATIO: f32 = 0.7;
pub(in crate::tui) const DIALOG_HEIGHT_RATIO: f32 = 0.6;
pub(in crate::tui) const LIST_DIALOG_WIDTH: u16 = 50;
pub(in crate::tui) const LIST_DIALOG_TALL_PADDING: u16 = 4;
pub(in crate::tui) const FIELD_DIALOG_DEFAULT_WIDTH: u16 = 60;

const GENERAL_LABELS: &[&str] = &[
    "API URL",
    "Template preset",
    "Instruct preset",
    "Reasoning preset",
    "TLS Skip Verify",
    "Debug Logging",
];
const GENERAL_BOOLEAN: &[usize] = &[4, 5];
const GENERAL_SELECTOR: &[usize] = &[1, 2, 3];

const SAMPLING_LABELS: &[&str] = &[
    "Temperature",
    "Top-K",
    "Top-P",
    "Min-P",
    "Repeat Last N",
    "Repeat Penalty",
    "Max Tokens",
];

const BACKUP_LABELS: &[&str] = &[
    "Enabled",
    "Keep All Days",
    "Keep Daily Days",
    "Keep Weekly Days",
    "Rebase Threshold %",
    "Rebase Hard Ceiling",
];
const BACKUP_BOOLEAN: &[usize] = &[0];

const SUMMARIZATION_LABELS: &[&str] = &[
    "Enabled",
    "API URL",
    "Context Size",
    "Trigger Threshold",
    "Prompt",
];
const SUMMARIZATION_BOOLEAN: &[usize] = &[0];
const SUMMARIZATION_MULTILINE: &[usize] = &[4];
const SUMMARIZATION_PLACEHOLDER: &[usize] = &[1];

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

pub fn open_config_editor(
    sections: Vec<Vec<String>>,
    locked: Vec<Vec<usize>>,
) -> TabbedFieldDialog<'static> {
    let [general_vals, sampling_vals, backup_vals, summarization_vals]: [Vec<String>; 4] =
        sections.try_into().expect("expected 4 section vectors");
    let [general_locked, sampling_locked, backup_locked, summarization_locked]: [Vec<usize>; 4] =
        locked.try_into().expect("expected 4 lock vectors");

    let general = TabSection::new("General", GENERAL_LABELS, general_vals)
        .with_boolean_fields(GENERAL_BOOLEAN)
        .with_selector_fields(GENERAL_SELECTOR)
        .with_locked_fields(general_locked);

    let sampling = TabSection::new("Sampling", SAMPLING_LABELS, sampling_vals)
        .with_locked_fields(sampling_locked)
        .with_validated_fields(vec![
            (0, FieldValidation::Float { min: 0.0, max: 2.0 }),
            (1, FieldValidation::Int { min: 1, max: 100 }),
            (2, FieldValidation::Float { min: 0.0, max: 1.0 }),
            (3, FieldValidation::Float { min: 0.0, max: 1.0 }),
            (4, FieldValidation::Int { min: -1, max: 32767 }),
            (5, FieldValidation::Float { min: 0.0, max: 2.0 }),
            (6, FieldValidation::Int { min: -1, max: 32767 }),
        ]);

    let backup = TabSection::new("Backup", BACKUP_LABELS, backup_vals)
        .with_boolean_fields(BACKUP_BOOLEAN)
        .with_locked_fields(backup_locked)
        .with_validated_fields(vec![
            (1, FieldValidation::Int { min: 0, max: 3650 }),
            (2, FieldValidation::Int { min: 0, max: 3650 }),
            (3, FieldValidation::Int { min: 0, max: 3650 }),
            (4, FieldValidation::Int { min: 0, max: 100 }),
            (5, FieldValidation::Int { min: 0, max: 100 }),
        ]);

    let summarization =
        TabSection::new("Summarization", SUMMARIZATION_LABELS, summarization_vals)
            .with_multiline_fields(SUMMARIZATION_MULTILINE)
            .with_boolean_fields(SUMMARIZATION_BOOLEAN)
            .with_placeholder(SUMMARIZATION_PLACEHOLDER, "(inherit main api_url)")
            .with_locked_fields(summarization_locked)
            .with_validated_fields(vec![
                (2, FieldValidation::Int { min: 512, max: 131072 }),
                (3, FieldValidation::Int { min: 1, max: 100 }),
            ]);

    TabbedFieldDialog::new(
        " Configuration ",
        vec![general, sampling, backup, summarization],
    )
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

const THEME_TAB_LABELS: &[&str] = &["Base theme", "", "Reset all colors", "Cancel"];
const THEME_TAB_SELECTOR: &[usize] = &[0];
const THEME_TAB_SEPARATOR: &[usize] = &[1];
const THEME_TAB_ACTIONS: &[usize] = &[2, 3];

pub(in crate::tui) const THEME_COLOR_TAB_LAYOUT: &[&[libllm::config::ColorLabel]] = &[
    MESSAGES_LABEL_IDS,
    BORDERS_STATUS_LABEL_IDS,
    UI_LABEL_IDS,
    INDICATORS_LABEL_IDS,
];

const MESSAGES_LABEL_IDS: &[libllm::config::ColorLabel] = &[
    libllm::config::ColorLabel::UserMessage,
    libllm::config::ColorLabel::AssistantMessageFg,
    libllm::config::ColorLabel::AssistantMessageBg,
    libllm::config::ColorLabel::SystemMessage,
    libllm::config::ColorLabel::Dialogue,
];
const MESSAGES_LABELS: &[&str] = &[
    libllm::config::ColorLabel::UserMessage.name(),
    libllm::config::ColorLabel::AssistantMessageFg.name(),
    libllm::config::ColorLabel::AssistantMessageBg.name(),
    libllm::config::ColorLabel::SystemMessage.name(),
    libllm::config::ColorLabel::Dialogue.name(),
];
const MESSAGES_COLOR_FIELDS: &[usize] = &[0, 1, 2, 3, 4];

const BORDERS_STATUS_LABEL_IDS: &[libllm::config::ColorLabel] = &[
    libllm::config::ColorLabel::BorderFocused,
    libllm::config::ColorLabel::BorderUnfocused,
    libllm::config::ColorLabel::StatusBarFg,
    libllm::config::ColorLabel::StatusBarBg,
    libllm::config::ColorLabel::StatusErrorFg,
    libllm::config::ColorLabel::StatusErrorBg,
    libllm::config::ColorLabel::StatusInfoFg,
    libllm::config::ColorLabel::StatusInfoBg,
    libllm::config::ColorLabel::StatusWarningFg,
    libllm::config::ColorLabel::StatusWarningBg,
];
const BORDERS_STATUS_LABELS: &[&str] = &[
    libllm::config::ColorLabel::BorderFocused.name(),
    libllm::config::ColorLabel::BorderUnfocused.name(),
    libllm::config::ColorLabel::StatusBarFg.name(),
    libllm::config::ColorLabel::StatusBarBg.name(),
    libllm::config::ColorLabel::StatusErrorFg.name(),
    libllm::config::ColorLabel::StatusErrorBg.name(),
    libllm::config::ColorLabel::StatusInfoFg.name(),
    libllm::config::ColorLabel::StatusInfoBg.name(),
    libllm::config::ColorLabel::StatusWarningFg.name(),
    libllm::config::ColorLabel::StatusWarningBg.name(),
];
const BORDERS_STATUS_COLOR_FIELDS: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9];

const UI_LABEL_IDS: &[libllm::config::ColorLabel] = &[
    libllm::config::ColorLabel::NavCursorFg,
    libllm::config::ColorLabel::NavCursorBg,
    libllm::config::ColorLabel::HoverBg,
    libllm::config::ColorLabel::SidebarHighlightFg,
    libllm::config::ColorLabel::SidebarHighlightBg,
    libllm::config::ColorLabel::Dimmed,
    libllm::config::ColorLabel::CommandPickerFg,
    libllm::config::ColorLabel::CommandPickerBg,
];
const UI_LABELS: &[&str] = &[
    libllm::config::ColorLabel::NavCursorFg.name(),
    libllm::config::ColorLabel::NavCursorBg.name(),
    libllm::config::ColorLabel::HoverBg.name(),
    libllm::config::ColorLabel::SidebarHighlightFg.name(),
    libllm::config::ColorLabel::SidebarHighlightBg.name(),
    libllm::config::ColorLabel::Dimmed.name(),
    libllm::config::ColorLabel::CommandPickerFg.name(),
    libllm::config::ColorLabel::CommandPickerBg.name(),
];
const UI_COLOR_FIELDS: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 7];

const INDICATORS_LABEL_IDS: &[libllm::config::ColorLabel] = &[
    libllm::config::ColorLabel::StreamingIndicator,
    libllm::config::ColorLabel::ApiUnavailable,
    libllm::config::ColorLabel::SummaryIndicator,
];
const INDICATORS_LABELS: &[&str] = &[
    libllm::config::ColorLabel::StreamingIndicator.name(),
    libllm::config::ColorLabel::ApiUnavailable.name(),
    libllm::config::ColorLabel::SummaryIndicator.name(),
];
const INDICATORS_COLOR_FIELDS: &[usize] = &[0, 1, 2];

pub fn open_theme_editor(config: &libllm::config::Config) -> TabbedFieldDialog<'static> {
    let overrides = config
        .theme_colors
        .as_ref()
        .cloned()
        .unwrap_or_default();

    let base_theme = config.theme.clone().unwrap_or_else(|| "dark".to_owned());

    let theme_vals = vec![
        base_theme,
        String::new(),
        String::new(),
        String::new(),
    ];
    let theme_tab = TabSection::new("Theme", THEME_TAB_LABELS, theme_vals)
        .with_selector_fields(THEME_TAB_SELECTOR)
        .with_action_fields(THEME_TAB_ACTIONS)
        .with_separator_fields(THEME_TAB_SEPARATOR);

    let messages_vals: Vec<String> = MESSAGES_LABEL_IDS
        .iter()
        .map(|l| overrides.get(*l).unwrap_or_default().to_owned())
        .collect();
    let messages = TabSection::new("Messages", MESSAGES_LABELS, messages_vals)
        .with_validated_fields(color_validations(MESSAGES_LABELS.len()))
        .with_color_preview_fields(MESSAGES_COLOR_FIELDS);

    let borders_vals: Vec<String> = BORDERS_STATUS_LABEL_IDS
        .iter()
        .map(|l| overrides.get(*l).unwrap_or_default().to_owned())
        .collect();
    let borders_status = TabSection::new("Borders & Status", BORDERS_STATUS_LABELS, borders_vals)
        .with_validated_fields(color_validations(BORDERS_STATUS_LABELS.len()))
        .with_color_preview_fields(BORDERS_STATUS_COLOR_FIELDS);

    let ui_vals: Vec<String> = UI_LABEL_IDS
        .iter()
        .map(|l| overrides.get(*l).unwrap_or_default().to_owned())
        .collect();
    let ui_tab = TabSection::new("UI", UI_LABELS, ui_vals)
        .with_validated_fields(color_validations(UI_LABELS.len()))
        .with_color_preview_fields(UI_COLOR_FIELDS);

    let ind_vals: Vec<String> = INDICATORS_LABEL_IDS
        .iter()
        .map(|l| overrides.get(*l).unwrap_or_default().to_owned())
        .collect();
    let indicators = TabSection::new("Indicators", INDICATORS_LABELS, ind_vals)
        .with_validated_fields(color_validations(INDICATORS_LABELS.len()))
        .with_color_preview_fields(INDICATORS_COLOR_FIELDS);

    TabbedFieldDialog::new(
        " Theme ",
        vec![theme_tab, messages, borders_status, ui_tab, indicators],
    )
}

fn color_validations(count: usize) -> Vec<(usize, FieldValidation)> {
    (0..count).map(|i| (i, FieldValidation::Color)).collect()
}

