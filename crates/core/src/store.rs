//! 設定と解析結果の保存。登録したリポジトリには何も書かない。
use crate::model::{Analysis, Chapter, ChapterKind, DeepDive, Settings, SkyPreview};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::io;
use std::path::{Path, PathBuf};

pub struct Store {
    root: PathBuf,
}

fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp.{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

impl Store {
    pub fn new(root: PathBuf) -> Store {
        Store { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn default_root() -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join("Library/Application Support/cc-code-view")
    }

    fn repo_dir(&self, repo: &Path) -> PathBuf {
        self.root.join("repos").join(hash(&repo.to_string_lossy()))
    }

    pub fn load_settings(&self) -> Settings {
        read_json(&self.root.join("settings.json")).unwrap_or_default()
    }

    pub fn save_settings(&self, settings: &Settings) -> io::Result<()> {
        write_json(&self.root.join("settings.json"), settings)
    }

    pub fn load_analysis(&self, repo: &Path) -> Option<Analysis> {
        read_json(&self.repo_dir(repo).join("analysis.json"))
    }

    pub fn save_analysis(&self, repo: &Path, analysis: &Analysis) -> io::Result<()> {
        write_json(&self.repo_dir(repo).join("analysis.json"), analysis)
    }

    pub fn load_preview(&self, repo: &Path) -> Option<SkyPreview> {
        read_json(&self.repo_dir(repo).join("preview.json"))
    }

    pub fn save_preview(&self, repo: &Path, preview: &SkyPreview) -> io::Result<()> {
        write_json(&self.repo_dir(repo).join("preview.json"), preview)
    }

    fn deep_path(&self, repo: &Path, module_id: &str) -> PathBuf {
        self.repo_dir(repo).join("deep").join(format!("{}.json", hash(module_id)))
    }

    pub fn load_deep_dive(&self, repo: &Path, module_id: &str) -> Option<DeepDive> {
        read_json(&self.deep_path(repo, module_id))
    }

    pub fn save_deep_dive(&self, repo: &Path, dive: &DeepDive) -> io::Result<()> {
        write_json(&self.deep_path(repo, &dive.module_id), dive)
    }

    fn chapter_path(&self, repo: &Path, kind: ChapterKind) -> PathBuf {
        self.repo_dir(repo).join("blueprint").join(format!("{}.json", kind.slug()))
    }

    /// 設計図の章。章ごとに別のファイルにし、書けた章から残す。
    pub fn load_chapter(&self, repo: &Path, kind: ChapterKind) -> Option<Chapter> {
        read_json::<Chapter>(&self.chapter_path(repo, kind)).filter(|c| c.kind == kind)
    }

    pub fn save_chapter(&self, repo: &Path, chapter: &Chapter) -> io::Result<()> {
        write_json(&self.chapter_path(repo, chapter.kind), chapter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelChoice;

    #[test]
    fn round_trips_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        assert_eq!(store.load_settings().last_model, ModelChoice::Default);
        let repo = Path::new("/x/repo");
        assert!(store.load_analysis(repo).is_none());
        let a = Analysis {
            head: Some("abc".into()),
            model: ModelChoice::Opus,
            overview: "o".into(),
            modules: vec![],
            tour: vec![],
        };
        store.save_analysis(repo, &a).unwrap();
        assert_eq!(store.load_analysis(repo).unwrap().model, ModelChoice::Opus);
        let d = DeepDive {
            module_id: "m/x".into(),
            head: None,
            model: ModelChoice::Haiku,
            explanation: "e".into(),
            key_files: vec![],
        };
        store.save_deep_dive(repo, &d).unwrap();
        assert_eq!(store.load_deep_dive(repo, "m/x").unwrap(), d);
        assert!(store.load_deep_dive(repo, "other").is_none());
        let c = Chapter {
            kind: ChapterKind::Domain,
            head: Some("abc".into()),
            model: ModelChoice::Sonnet,
            generated_at: 1,
            headline: "h".into(),
            summary: "s".into(),
            diagrams: vec![],
            points: vec![],
            glossary: vec![],
        };
        store.save_chapter(repo, &c).unwrap();
        assert_eq!(store.load_chapter(repo, ChapterKind::Domain).unwrap(), c);
        assert!(store.load_chapter(repo, ChapterKind::Modules).is_none());
        let p = SkyPreview { stars: vec![(0.5, 0.5, 2.0, 1.0)], lines: vec![] };
        store.save_preview(repo, &p).unwrap();
        assert_eq!(store.load_preview(repo).unwrap(), p);
    }

    #[test]
    fn corrupt_file_reads_as_missing_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        std::fs::write(dir.path().join("settings.json"), "{broken").unwrap();
        assert!(store.load_settings().repos.is_empty());
        store.save_settings(&Settings::default()).unwrap();
        let leftovers = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn model_aliases_match_the_cli() {
        assert_eq!(ModelChoice::Default.cli_alias(), None);
        assert_eq!(ModelChoice::Fable.cli_alias(), Some("fable"));
        assert_eq!(ModelChoice::Opus.cli_alias(), Some("opus"));
        assert_eq!(ModelChoice::Sonnet.cli_alias(), Some("sonnet"));
        assert_eq!(ModelChoice::Haiku.cli_alias(), Some("haiku"));
    }
}
