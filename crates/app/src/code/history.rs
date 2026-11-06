//! 移動履歴。

pub struct History<T> {
    items: Vec<T>,
    at: usize,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        History { items: Vec::new(), at: 0 }
    }
}

impl<T> History<T> {
    pub fn push(&mut self, item: T) {
        if !self.items.is_empty() {
            self.items.truncate(self.at + 1);
        }
        self.items.push(item);
        self.at = self.items.len() - 1;
    }

    pub fn back(&mut self) -> Option<&T> {
        if self.at == 0 {
            return None;
        }
        self.at -= 1;
        self.items.get(self.at)
    }

    pub fn forward(&mut self) -> Option<&T> {
        if self.at + 1 >= self.items.len() {
            return None;
        }
        self.at += 1;
        self.items.get(self.at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_truncates_forward_on_push() {
        let mut h = History::default();
        h.push(1);
        h.push(2);
        h.push(3);
        assert_eq!(h.back(), Some(&2));
        h.push(9);
        assert_eq!(h.forward(), None);
        assert_eq!(h.back(), Some(&2));
        assert_eq!(h.forward(), Some(&9));
    }

    #[test]
    fn empty_history_is_safe() {
        let mut h: History<u8> = History::default();
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), None);
    }
}
