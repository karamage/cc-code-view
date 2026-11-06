//! ガイドツアーの進行。
use ccv_core::model::{RepoIndex, TourStop};

pub struct TourPlayer {
    stops: Vec<TourStop>,
    valid: Vec<bool>,
    position: usize,
}

/// 消えたファイルや、短くなったファイルを指す停留所は無効にする。
pub fn validity(stops: &[TourStop], index: &RepoIndex) -> Vec<bool> {
    stops
        .iter()
        .map(|s| index.files.iter().any(|f| f.path == s.path && s.start_line <= f.lines))
        .collect()
}

/// 保存された位置を、有効な停留所に合わせる。
pub fn resume_at(count: usize, valid: &[bool], saved: usize) -> usize {
    let saved = saved.min(count.saturating_sub(1));
    (saved..count)
        .find(|i| valid[*i])
        .or_else(|| (0..saved).rev().find(|i| valid[*i]))
        .unwrap_or(0)
}

impl TourPlayer {
    pub fn new(stops: Vec<TourStop>, valid: Vec<bool>, saved: usize) -> TourPlayer {
        let position = resume_at(stops.len(), &valid, saved);
        TourPlayer { stops, valid, position }
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn len(&self) -> usize {
        self.stops.len()
    }

    pub fn current(&self) -> Option<&TourStop> {
        self.stops.get(self.position).filter(|_| self.valid.get(self.position) == Some(&true))
    }

    pub fn next(&mut self) -> Option<&TourStop> {
        let found = (self.position + 1..self.stops.len()).find(|i| self.valid[*i])?;
        self.position = found;
        self.stops.get(found)
    }

    pub fn prev(&mut self) -> Option<&TourStop> {
        let found = (0..self.position).rev().find(|i| self.valid[*i])?;
        self.position = found;
        self.stops.get(found)
    }

    /// ここまでに通過した有効な停留所のパス。航路を描くのに使う。
    pub fn visited(&self) -> Vec<&str> {
        (0..=self.position.min(self.stops.len().saturating_sub(1)))
            .filter(|i| self.valid.get(*i) == Some(&true))
            .map(|i| self.stops[i].path.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(t: &str) -> TourStop {
        TourStop { title: t.into(), body: String::new(), path: "a".into(), start_line: 1, end_line: 1 }
    }

    #[test]
    fn skips_invalid_stops_and_stops_at_ends() {
        let mut p = TourPlayer::new(vec![stop("a"), stop("b"), stop("c")], vec![true, false, true], 0);
        assert_eq!(p.next().unwrap().title, "c");
        assert!(p.next().is_none());
        assert_eq!(p.position(), 2);
        assert_eq!(p.visited().len(), 2);
        assert_eq!(p.prev().unwrap().title, "a");
        assert!(p.prev().is_none());
    }

    #[test]
    fn resume_position_is_clamped_to_a_valid_stop() {
        assert_eq!(resume_at(3, &[true, true, false], 99), 1);
        assert_eq!(resume_at(3, &[false, true, true], 0), 1);
        assert_eq!(resume_at(0, &[], 5), 0);
    }

    #[test]
    fn empty_or_all_invalid_tour_has_no_current() {
        let p = TourPlayer::new(vec![], vec![], 0);
        assert!(p.current().is_none());
        let p = TourPlayer::new(vec![stop("a")], vec![false], 0);
        assert!(p.current().is_none());
    }

    #[test]
    fn validity_checks_file_and_length() {
        use ccv_core::model::{FileEntry, Lang};
        let index = RepoIndex {
            files: vec![FileEntry { path: "a".into(), lang: Lang::Other, lines: 5, bytes: 0, binary: false, churn: 0 }],
            ..Default::default()
        };
        let mut late = stop("x");
        late.start_line = 9;
        let mut gone = stop("y");
        gone.path = "b".into();
        assert_eq!(validity(&[stop("ok"), late, gone], &index), [true, false, false]);
    }
}
