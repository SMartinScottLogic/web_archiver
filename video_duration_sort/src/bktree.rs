use crate::media::image::Fingerprint;

#[derive(Debug)]
pub struct BkTree<T> {
    root: Option<Box<Node<T>>>,
}

#[derive(Debug)]
struct Node<T> {
    value: T,
    fp: Fingerprint,
    children: Vec<(u32, Box<Node<T>>)>,
}

impl<T> BkTree<T> {
    pub fn new() -> Self {
        Self { root: None }
    }

    pub fn insert(&mut self, fp: Fingerprint, value: T) {
        match &mut self.root {
            None => {
                self.root = Some(Box::new(Node {
                    value,
                    fp,
                    children: Vec::new(),
                }));
            }

            Some(root) => {
                root.insert(fp, value);
            }
        }
    }

    pub fn search<'a>(&'a self, target: &Fingerprint, max_dist: u32, results: &mut Vec<&'a T>) {
        if let Some(root) = &self.root {
            root.search(target, max_dist, results);
        }
    }
}

impl<T> Node<T> {
    fn insert(&mut self, fp: Fingerprint, value: T) {
        let d = self.fp.distance(&fp);

        for (edge, child) in &mut self.children {
            if *edge == d {
                child.insert(fp, value);
                return;
            }
        }

        self.children.push((
            d,
            Box::new(Node {
                value,
                fp,
                children: Vec::new(),
            }),
        ));
    }

    fn search<'a>(&'a self, target: &Fingerprint, max_dist: u32, results: &mut Vec<&'a T>) {
        let d = self.fp.distance(target);

        if d <= max_dist {
            results.push(&self.value);
        }

        let lower = d.saturating_sub(max_dist);
        let upper = d + max_dist;

        for (edge, child) in &self.children {
            if *edge >= lower && *edge <= upper {
                child.search(target, max_dist, results);
            }
        }
    }
}
