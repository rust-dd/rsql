//! Language editing configuration. Grammar resources live in `highlighter`.

pub use gpui_base::input::language_config::LanguageConfig;

use gpui_base::input::{AutoClosingPair, BracketPair, IndentationRules, SyntaxContext};
use regex::Regex;
use std::sync::LazyLock;

struct ComponentLanguages {
    defaults: std::collections::HashMap<&'static str, std::rc::Rc<LanguageConfig>>,
    fallback: std::rc::Rc<LanguageConfig>,
}

impl gpui_base::input::LanguageProvider for ComponentLanguages {
    fn language_name(&self, name: &str) -> gpui::SharedString {
        crate::highlighter::LanguageRegistry::singleton().editing_language_name(name)
    }

    fn config(&self, name: &str) -> std::rc::Rc<LanguageConfig> {
        self.defaults.get(name).unwrap_or(&self.fallback).clone()
    }

    fn syntax_context_provider(
        &self,
        name: &str,
    ) -> Option<std::rc::Rc<dyn gpui_base::input::SyntaxContextProvider>> {
        super::syntax_context::syntax_context_provider(name)
    }
}

pub(crate) fn init(cx: &mut gpui::App) {
    gpui_base::input::set_language_provider(
        std::rc::Rc::new(ComponentLanguages {
            defaults: ["text", "json", "python"]
                .into_iter()
                .map(|name| (name, std::rc::Rc::new(default_language_config(name))))
                .collect(),
            fallback: std::rc::Rc::new(LanguageConfig::default()),
        }),
        cx,
    );
}

/// Language editing defaults. Editor preferences are stored separately on EditorState.
/// Unknown languages use structural bracket indentation; only Python treats a colon
/// as a block opener. Plain text has no structural or automatic pairs.
fn default_language_config(language: &str) -> LanguageConfig {
    static PYTHON_INDENT: LazyLock<IndentationRules> = LazyLock::new(|| {
        IndentationRules::new(
            Regex::new(r"[\{\(\[:]\s*$").unwrap(),
            Regex::new(r"^\s*[\}\)\]]").unwrap(),
        )
    });
    match language.to_lowercase().as_str() {
        "text" => LanguageConfig::default()
            .brackets([])
            .auto_closing_pairs([]),
        "json" => LanguageConfig::default()
            .brackets([BracketPair::new("{", "}"), BracketPair::new("[", "]")])
            .auto_closing_pairs(
                [
                    AutoClosingPair::new("{", "}"),
                    AutoClosingPair::new("[", "]"),
                    AutoClosingPair::new("\"", "\""),
                ]
                .into_iter()
                .map(|p| p.not_in([SyntaxContext::String, SyntaxContext::Comment])),
            ),
        "python" => LanguageConfig::default().indentation_rules(PYTHON_INDENT.clone()),
        _ => LanguageConfig::default(),
    }
}
