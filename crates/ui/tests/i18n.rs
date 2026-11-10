//! Translation coverage and live preference changes, without screenshot baselines.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]
use dms_engine::command::{COMMANDS, Menu};
use dms_exec::schema::{self, ParamKind};
use dms_ui::i18n::{self, Language, tr, tr_fmt};
use dms_ui::{Platform, ShellState, Workbench};
use egui::{Key, Modifiers, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, PartialEq)]
enum Token {
    Name(String),
    Text(String),
    Punct(char),
}

/// Skip comments and string bodies, so examples/comments cannot silently add translation keys.
fn source_keys(source: &str) -> BTreeSet<String> {
    let mut rest = source.split("#[cfg(test)]").next().unwrap();
    let mut tokens = Vec::new();
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("//") {
            rest = rest.split_once('\n').map_or("", |(_, tail)| tail);
        } else if let Some(tail) = rest.strip_prefix("/*") {
            rest = tail;
            let mut depth = 1usize;
            while depth > 0 && !rest.is_empty() {
                if let Some(tail) = rest.strip_prefix("/*") {
                    depth += 1;
                    rest = tail;
                } else if let Some(tail) = rest.strip_prefix("*/") {
                    depth -= 1;
                    rest = tail;
                } else {
                    rest = &rest[rest.chars().next().unwrap().len_utf8()..];
                }
            }
        } else if c == '"' {
            let mut escaped = false;
            let end = rest
                .char_indices()
                .skip(1)
                .find_map(|(index, c)| {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        return Some(index + 1);
                    }
                    None
                })
                .unwrap();
            // UI keys use the common JSON/TOML/Rust basic-string escape subset.
            // Other Rust literals may contain escapes specific to Rust; ignore their contents.
            if let Ok(text) = serde_json::from_str(&rest[..end]) {
                tokens.push(Token::Text(text));
            } else {
                tokens.push(Token::Punct('"'));
            }
            rest = &rest[end..];
        } else if c.is_alphabetic() || c == '_' {
            let end = rest
                .char_indices()
                .find(|(_, c)| !c.is_alphanumeric() && *c != '_')
                .map_or(rest.len(), |(index, _)| index);
            tokens.push(Token::Name(rest[..end].into()));
            rest = &rest[end..];
        } else {
            if !c.is_whitespace() {
                tokens.push(Token::Punct(c));
            }
            rest = &rest[c.len_utf8()..];
        }
    }
    tokens
        .windows(3)
        .filter_map(|tokens| match tokens {
            [Token::Name(name), Token::Punct('('), Token::Text(text)]
                if name == "tr" || name == "tr_fmt" =>
            {
                Some(text.clone())
            }
            _ => None,
        })
        .collect()
}

fn kind_keys(kind: &ParamKind, keys: &mut BTreeSet<String>) {
    match kind {
        ParamKind::Choice(choices) => {
            keys.extend(choices.iter().map(|(_, label)| label.to_string()))
        }
        ParamKind::Table { columns, .. } => {
            for column in *columns {
                keys.insert(column.label.into());
                kind_keys(&column.kind, keys);
            }
        }
        _ => {}
    }
}

#[test]
fn translation_keys_exactly_cover_sources_commands_menus_and_every_schema() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut keys = BTreeSet::new();
    let paths = std::fs::read_dir(root.join("crates/ui/src"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .chain([
            root.join("apps/dms/src/main.rs"),
            root.join("apps/dms/src/macos_menu.rs"),
        ]);
    for path in paths {
        keys.extend(source_keys(&std::fs::read_to_string(path).unwrap()));
    }
    keys.extend(COMMANDS.iter().map(|spec| spec.label.to_string()));
    keys.extend(Menu::ALL.iter().map(|menu| menu.label().to_string()));
    for node in dms_engine::catalog::NODES {
        if let Some(specs) = schema::schema(node.id) {
            for spec in specs {
                keys.extend([spec.label.into(), spec.help.into(), spec.group.into()]);
                kind_keys(&spec.kind, &mut keys);
            }
        }
    }
    let table = i18n::parse_table(i18n::TRANSLATIONS).unwrap();
    let actual: BTreeSet<_> = table.keys().cloned().collect();
    if actual != keys {
        eprintln!("I18N_KEYS={}", serde_json::to_string(&keys).unwrap());
    }
    assert_eq!(actual, keys, "Translation keys must match exactly");
    let ordered: Vec<String> = i18n::TRANSLATIONS
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let pair = i18n::parse_table(line).expect("one quoted key/value per line");
            assert_eq!(pair.len(), 1);
            pair.into_keys().next().unwrap()
        })
        .collect();
    assert_eq!(
        ordered,
        actual.into_iter().collect::<Vec<_>>(),
        "Keys must be sorted"
    );
}

#[test]
fn source_scanner_ignores_comments_and_literal_contents() {
    let source = r#"// tr("comment")
        /* tr_fmt("comment", &[]) */
        let example = "tr(\"example\")";
        tr("Real"); tr_fmt("{count} rows", &[]); tr(spec.label);
    "#;
    assert_eq!(
        source_keys(source),
        BTreeSet::from(["Real".into(), "{count} rows".into()])
    );
}

// One test owns process language changes; the other tests only read the compiled table.
#[test]
fn live_language_settings_menus_toolbar_and_undo_restore_english() {
    assert_eq!(
        i18n::language(),
        Language::English,
        "library default is English"
    );
    let old: ShellState = serde_json::from_str("{}").unwrap();
    assert_eq!(old.language, Language::System);
    assert_eq!(tr("Open…"), "Open…");
    i18n::set_language(Language::SimplifiedChinese);
    assert_eq!(tr("An untranslated test key"), "An untranslated test key");
    for (source, value) in [
        ("Open…", "打开…"),
        ("File", "文件"),
        ("Run", "运行"),
        ("Undo", "撤销"),
        ("Record condition", "记录条件"),
        ("Change {label}", "修改{label}"),
        ("{command} {label}", "{command} {label}"),
        ("Preview: {label}", "预览：{label}"),
    ] {
        i18n::set_translation_for_test(source, Some(value));
    }
    assert_eq!(tr("Open…"), "打开…");
    assert_eq!(
        tr_fmt("Preview: {label}", &[("label", "{label} 用户数据")]),
        "预览：{label} 用户数据"
    );
    i18n::set_language(Language::English);
    let mut w = Workbench::new(ShellState {
        data_folders: vec!["/tmp/data-one".into(), "/tmp/data-two".into()],
        ..Default::default()
    });
    w.dispatch(&egui::Context::default(), "app.settings", &mut |_| None);
    let mut h = Harness::builder()
        .with_size(vec2(1400.0, 1000.0))
        .with_os(egui::os::OperatingSystem::Mac)
        .build_ui_state(
            |ui, w: &mut Workbench| {
                // Settings includes the language autonym even while the UI is English.
                ui.ctx()
                    .set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
                for id in w.ui(
                    ui,
                    &Platform {
                        mac: true,
                        ..Default::default()
                    },
                ) {
                    w.dispatch(ui.ctx(), id, &mut |_| Some(std::env::temp_dir()));
                }
            },
            w,
        );
    h.run();
    assert!(h.query_all_by_label("Open…").count() >= 1);
    h.get_by_label("File");
    h.get_by_label("Language").click();
    h.run();
    h.get_by_label("简体中文").click();
    h.run();
    assert!(h.query_all_by_label("打开…").count() >= 1);
    h.get_by_label("文件");
    assert!(h.query_all_by_label("运行").count() >= 2);
    assert!(h.query_by_label("Open…").is_none());
    assert_eq!(h.state().state.language, Language::SimplifiedChinese);
    assert_eq!(
        serde_json::from_str::<ShellState>(&serde_json::to_string(&h.state().state).unwrap())
            .unwrap()
            .language,
        Language::SimplifiedChinese
    );
    h.get_all_by_label(tr("Remove")).next().unwrap().click();
    h.run();
    assert_eq!(
        h.state().state.data_folders,
        vec![std::path::PathBuf::from("/tmp/data-two")]
    );
    assert_eq!(
        h.state().session.data_folders(),
        h.state().state.data_folders
    );
    h.get_by_label(tr("Add…")).click();
    h.run();
    assert_eq!(
        h.state().state.data_folders.last().unwrap(),
        std::env::temp_dir().as_path()
    );
    assert_eq!(
        h.state().session.data_folders(),
        h.state().state.data_folders
    );
    h.get_by_label(tr("Language")).click();
    h.run();
    h.get_by_label("English").click();
    h.run();
    assert!(h.query_all_by_label("Open…").count() >= 1);
    h.get_by_label("File");
    assert!(h.query_by_label("打开…").is_none());
    assert_eq!(h.state().state.language, Language::English);

    let mut stream = dms_ui::synthetic::stream(1);
    let node = &mut stream.diagram.nodes[0];
    node.class = "test.SelectNode".into();
    let uid = node.uid.clone();
    let mut session = dms_engine::session::Session::with_document(
        dms_engine::session::Document::from_stream("test.str".into(), stream),
    );
    session
        .execute(
            "node.set_param",
            &[
                dms_engine::session::Arg::Node(uid),
                dms_engine::session::Arg::Param(
                    "condition".into(),
                    schema::ParamValue::Text("x > 0".into()),
                ),
            ],
        )
        .unwrap();
    let undo = dms_engine::command::by_id("edit.undo").unwrap();
    assert_eq!(
        dms_ui::menu::command_label(&session, undo),
        "Undo Change Record condition"
    );
    i18n::set_language(Language::SimplifiedChinese);
    assert_eq!(
        dms_ui::menu::command_label(&session, undo),
        "撤销 修改记录条件"
    );
    i18n::set_language(Language::English);
    assert_eq!(
        dms_ui::menu::command_label(&session, undo),
        "Undo Change Record condition"
    );
    i18n::set_system_language(Some("zh-Hans-CN"));
    i18n::set_language(Language::System);
    assert_eq!(tr("Open…"), "打开…");
    i18n::set_system_language(Some("en-US"));
    i18n::set_language(Language::System);
    assert_eq!(tr("Open…"), "Open…");
    i18n::set_system_language(None);
    i18n::set_language(Language::English);
    for source in [
        "Open…",
        "File",
        "Run",
        "Undo",
        "Record condition",
        "Change {label}",
        "{command} {label}",
        "Preview: {label}",
    ] {
        i18n::set_translation_for_test(source, None);
    }
    // Cmd+, is available without an open document.
    assert!(h.state().session.is_enabled("app.settings"));
    let mut keys = Harness::builder()
        .with_os(egui::os::OperatingSystem::Mac)
        .build_ui_state(
            |ui, shortcuts: &mut Vec<&'static str>| {
                shortcuts.extend(dms_ui::menu::take_shortcuts(ui.ctx(), true));
            },
            vec![],
        );
    keys.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
    keys.run();
    assert!(keys.state().contains(&"app.settings"));
}
