use termimad::MadSkin;

pub fn render_markdown(text: &str) {
    let skin = MadSkin::default();
    skin.print_text(text);
}
