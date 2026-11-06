//! 出力の JSON スキーマと検証。AI の出力は信頼できない入力として扱う。
use ccv_core::model::{Analysis, DeepDive, KeyFile, Module, ModelChoice, RepoIndex, TourStop};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Component, Path};

pub const ANALYSIS_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"required":["overview","modules","tour"],"properties":{
"overview":{"type":"string"},
"modules":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["id","name","summary","paths"],"properties":{"id":{"type":"string"},"name":{"type":"string"},"summary":{"type":"string"},"paths":{"type":"array","items":{"type":"string"}}}}},
"tour":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["title","body","path","start_line","end_line"],"properties":{"title":{"type":"string"},"body":{"type":"string"},"path":{"type":"string"},"start_line":{"type":"integer"},"end_line":{"type":"integer"}}}}
}}"#;

pub const DEEP_DIVE_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"required":["explanation","key_files"],"properties":{
"explanation":{"type":"string"},
"key_files":{"type":"array","items":{"type":"object","additionalProperties":false,"required":["path","why"],"properties":{"path":{"type":"string"},"why":{"type":"string"}}}}
}}"#;

#[derive(Debug, PartialEq)]
pub struct InvalidOutput(pub String);

fn is_safe_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path).components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn clean_path(path: &str) -> String {
    path.trim_start_matches("./").trim_end_matches('/').to_string()
}

#[derive(Deserialize)]
struct RawAnalysis {
    overview: String,
    modules: Vec<Module>,
    tour: Vec<RawStop>,
}

#[derive(Deserialize)]
struct RawStop {
    title: String,
    body: String,
    path: String,
    start_line: i64,
    end_line: i64,
}

pub fn validate_analysis(
    raw: &Value,
    index: &RepoIndex,
    model: ModelChoice,
) -> Result<Analysis, InvalidOutput> {
    let parsed: RawAnalysis =
        serde_json::from_value(raw.clone()).map_err(|e| InvalidOutput(e.to_string()))?;
    let lines: HashMap<&str, u32> = index.files.iter().map(|f| (f.path.as_str(), f.lines)).collect();

    let path_exists = |p: &str| {
        lines.contains_key(p)
            || index.files.iter().any(|f| {
                f.path.starts_with(p) && f.path.as_bytes().get(p.len()) == Some(&b'/')
            })
    };
    let modules: Vec<Module> = parsed
        .modules
        .into_iter()
        .filter_map(|mut m| {
            m.paths = m
                .paths
                .iter()
                .filter(|p| is_safe_relative(p))
                .map(|p| clean_path(p))
                .filter(|p| path_exists(p))
                .collect();
            (!m.paths.is_empty()).then_some(m)
        })
        .collect();
    if modules.is_empty() {
        return Err(InvalidOutput("有効なモジュールがありません".into()));
    }

    let tour = parsed
        .tour
        .into_iter()
        .filter_map(|s| {
            if !is_safe_relative(&s.path) {
                return None;
            }
            let path = clean_path(&s.path);
            let total = *lines.get(path.as_str())?;
            if total == 0 || s.start_line < 1 || s.end_line < s.start_line || s.start_line > total as i64 {
                return None;
            }
            Some(TourStop {
                title: s.title,
                body: s.body,
                path,
                start_line: s.start_line as u32,
                end_line: (s.end_line.min(total as i64)) as u32,
            })
        })
        .collect();

    Ok(Analysis { head: index.head.clone(), model, overview: parsed.overview, modules, tour })
}

#[derive(Deserialize)]
struct RawDeepDive {
    explanation: String,
    key_files: Vec<KeyFile>,
}

pub fn validate_deep_dive(
    raw: &Value,
    index: &RepoIndex,
    module_id: &str,
    model: ModelChoice,
) -> Result<DeepDive, InvalidOutput> {
    let parsed: RawDeepDive =
        serde_json::from_value(raw.clone()).map_err(|e| InvalidOutput(e.to_string()))?;
    if parsed.explanation.trim().is_empty() {
        return Err(InvalidOutput("解説が空です".into()));
    }
    let key_files = parsed
        .key_files
        .into_iter()
        .filter(|k| is_safe_relative(&k.path))
        .map(|k| KeyFile { path: clean_path(&k.path), why: k.why })
        .filter(|k| index.files.iter().any(|f| f.path == k.path))
        .collect();
    Ok(DeepDive {
        module_id: module_id.to_string(),
        head: index.head.clone(),
        model,
        explanation: parsed.explanation,
        key_files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ccv_core::model::{FileEntry, Lang};
    use serde_json::json;

    fn index() -> RepoIndex {
        let f = |p: &str, lines| FileEntry {
            path: p.into(),
            lang: Lang::Rust,
            lines,
            bytes: 0,
            binary: false,
            churn: 0,
        };
        RepoIndex {
            head: Some("sha".into()),
            files: vec![f("src/a.rs", 100), f("src/b.rs", 10)],
            symbols: Default::default(),
            edges: vec![],
        }
    }

    #[test]
    fn drops_invalid_tour_stops_and_clamps_lines() {
        let raw = json!({"overview":"o","modules":[{"id":"m","name":"M","summary":"s","paths":["src/","../evil","nope","/etc"]}],
          "tour":[
            {"title":"ok","body":"b","path":"src/a.rs","start_line":10,"end_line":500},
            {"title":"missing","body":"b","path":"gone.rs","start_line":1,"end_line":2},
            {"title":"escape","body":"b","path":"../../etc/passwd","start_line":1,"end_line":2},
            {"title":"abs","body":"b","path":"/etc/passwd","start_line":1,"end_line":2},
            {"title":"reversed","body":"b","path":"src/a.rs","start_line":9,"end_line":3},
            {"title":"past end","body":"b","path":"src/b.rs","start_line":99,"end_line":100}
          ]});
        let a = validate_analysis(&raw, &index(), ModelChoice::Haiku).unwrap();
        assert_eq!(a.tour.len(), 1);
        assert_eq!(a.tour[0].end_line, 100);
        assert_eq!(a.modules[0].paths, ["src"]);
        assert_eq!(a.model, ModelChoice::Haiku);
        assert_eq!(a.head.as_deref(), Some("sha"));
    }

    #[test]
    fn rejects_wrong_shape_and_empty_results() {
        let m = ModelChoice::Default;
        assert!(validate_analysis(&json!({"overview":1}), &index(), m).is_err());
        assert!(validate_analysis(&json!({"overview":"o","modules":[],"tour":[]}), &index(), m).is_err());
    }

    #[test]
    fn deep_dive_filters_unknown_files() {
        let raw = json!({"explanation":"e","key_files":[{"path":"src/a.rs","why":"w"},{"path":"../x","why":"w"}]});
        let d = validate_deep_dive(&raw, &index(), "m", ModelChoice::Opus).unwrap();
        assert_eq!(d.key_files.len(), 1);
        assert_eq!(d.module_id, "m");
        assert!(validate_deep_dive(&json!({"explanation":" ","key_files":[]}), &index(), "m", ModelChoice::Opus).is_err());
    }

    #[test]
    fn schemas_are_valid_json() {
        for s in [ANALYSIS_SCHEMA, DEEP_DIVE_SCHEMA] {
            assert!(serde_json::from_str::<Value>(s).is_ok());
        }
    }
}
