use gpui_kit::{
    component::{Colorize, Theme, highlighter::SyntaxColors},
    *,
};
use rsql_desktop::completion::SuggestionKind;
use serde_json::json;
use std::sync::{Arc, Once};

fn hex(value: &str) -> Hsla {
    Hsla::parse_hex(value).expect("valid RSQL editor color")
}

#[derive(Clone, Copy)]
pub(in crate::workspace) struct SuggestionColors {
    pub background: Hsla,
    pub border: Hsla,
    pub foreground: Hsla,
    pub selected_background: Hsla,
    pub selected_foreground: Hsla,
    pub selected_icon: Hsla,
    pub highlight: Hsla,
    pub selected_highlight: Hsla,
    pub description: Hsla,
    column: Hsla,
    relation: Hsla,
    function: Hsla,
    schema: Hsla,
    keyword: Hsla,
}

impl SuggestionColors {
    pub fn new(dark: bool) -> Self {
        let pair = |light, night| hex(if dark { night } else { light });
        Self {
            background: pair("#ffffff", "#222040"),
            border: pair("#e2e2e8", "#3a3860"),
            foreground: pair("#1a1a2e", "#e4e1f4"),
            selected_background: pair("#f0f0f4", "#3a3768"),
            selected_foreground: pair("#1a1a2e", "#ffffff"),
            selected_icon: pair("#7c3aed", "#c4b5fd"),
            highlight: pair("#6d28d9", "#c4b5fd"),
            selected_highlight: pair("#6d28d9", "#ddd6fe"),
            description: pair("#5a5a70", "#a9a4c4"),
            column: pair("#0f766e", "#5eead4"),
            relation: pair("#b45309", "#fbbf24"),
            function: pair("#7c3aed", "#c4b5fd"),
            schema: pair("#2563eb", "#7dd3fc"),
            keyword: pair("#666680", "#a9a4c4"),
        }
    }

    pub fn icon(&self, kind: Option<SuggestionKind>, selected: bool) -> Hsla {
        if selected {
            return self.selected_icon;
        }
        match kind {
            Some(SuggestionKind::Column) => self.column,
            Some(SuggestionKind::Relation) => self.relation,
            Some(SuggestionKind::Function) => self.function,
            Some(SuggestionKind::Schema) => self.schema,
            _ => self.keyword,
        }
    }
}

fn register_sql_captures() {
    use gpui_kit::component::highlighter::{GrammarConfig, LanguageRegistry};
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        // Sequel gives every literal a string capture, including its more specific numeric captures.
        let number = r"^[-+]?([0-9]+(_[0-9]+)*([.]([0-9]+(_[0-9]+)*)?)?|[.][0-9]+(_[0-9]+)*)([eE][-+]?[0-9]+(_[0-9]+)*)?$";
        let literals = format!(r#"((literal) @number (#match? @number "{number}"))
            ((literal) @string (#not-match? @string "{number}"))"#);
        let query = tree_sitter_sequel::HIGHLIGHTS_QUERY
            .replace("(literal) @string", &literals)
            .replace("@float", "@number")
            .replace("@type)", "@variable)")
            .replace("@conditional", "@keyword")
            .replace("@storageclass", "@keyword")
            .replace("@keyword.operator", "@operator")
            .replace("@field", "@property")
            .replace("@parameter", "@variable");
        let config = GrammarConfig::new("sql", tree_sitter_sequel::LANGUAGE.into(), vec![], &query, "", "");
        LanguageRegistry::singleton().register("sql", &config);
    });
}

pub(super) fn scrollbar(mode: gpui_kit::component::ThemeMode, cx: &mut App) {
    let active = hex(if mode.is_dark() {
        "#4a486080"
    } else {
        "#9999a880"
    });
    let scrollbar = &mut gpui_kit::base::Theme::global_mut(cx).scrollbar;
    let styles = scrollbar
        .styles()
        .clone()
        .thumb_active(|style| style.bg(active));
    *scrollbar = scrollbar.clone().with_styles(styles);
}

pub(super) fn apply(theme: &mut Theme, dark: bool) {
    register_sql_captures();
    let pair = |light, night| if dark { night } else { light };
    let background = hex(pair("#fefefe", "#1c1a2e"));
    let foreground = pair("#1a1a2e", "#d4d0e8");
    let keyword = pair("#7c3aed", "#a78bfa");
    let comment = pair("#9999a8", "#6b6b80");
    let predefined = pair("#2563eb", "#60a5fa");
    let punctuation = pair("#666680", "#8888a0");
    let style = &mut Arc::make_mut(&mut theme.highlight_theme).style;
    style.editor_background = Some(background);
    style.editor_gutter_background = Some(background);
    style.editor_foreground = Some(hex(foreground));
    style.editor_line_number = Some(hex(pair("#9999a8", "#4a4860")));
    style.editor_active_line_number = Some(hex(punctuation));
    style.editor_active_line = Some(hex(pair("#f5f5f8", "#252340")));
    style.syntax = serde_json::from_value::<SyntaxColors>(json!({
        "keyword": {"color": keyword, "font_weight": 700},
        "boolean": {"color": keyword, "font_weight": 700},
        "operator": {"color": keyword},
        "string": {"color": pair("#059669", "#34d399")},
        "number": {"color": pair("#b45309", "#fbbf24")},
        "comment": {"color": comment, "font_style": "italic"},
        "variable": {"color": foreground}, "property": {"color": foreground},
        "attribute": {"color": foreground}, "type": {"color": predefined},
        "function": {"color": predefined},
        "punctuation": {"color": punctuation}, "punctuation.bracket": {"color": punctuation},
        "punctuation.delimiter": {"color": punctuation}
    }))
    .expect("valid SQL highlight theme");
    theme.selection = hex(pair("#ddd6fe80", "#7c3aed40"));
    theme.caret = hex(keyword);
    theme.scrollbar_thumb = hex(pair("#e2e2e840", "#2a284540"));
    theme.scrollbar_thumb_hover = hex(pair("#c4c4d060", "#3a386060"));
}

pub(in crate::workspace) fn local_paint(dark: bool) -> gpui_kit::base::input::EditorPaint {
    use gpui_kit::base::input::{EditorPaint, EditorSurface};
    let pair = |a, b| hex(if dark { b } else { a });
    let widget = EditorSurface {
        fill: pair("#ffffff", "#222040"),
        border: pair("#e2e2e8", "#2a2845"),
        foreground: pair("#1a1a2e", "#e4e1f4"),
    };
    EditorPaint {
        inactive_selection: pair("#e2e2e840", "#7c3aed20"),
        line_number: pair("#9999a8", "#4a4860"),
        active_line_number: pair("#666680", "#8888a0"),
        guide: pair("#e2e2e8", "#2a2845"),
        active_guide: pair("#c4c4d0", "#3a3860"),
        widget,
        hover: EditorSurface {
            border: pair("#e2e2e8", "#3a3860"),
            ..widget
        },
        input: EditorSurface {
            fill: pair("#f5f5f8", "#1c1a2e"),
            ..widget
        },
    }
}
