// Tree is always non-empty
pub struct Tree<T> {
    value: T,
    children: Vec<Tree<T>>,
}

impl<T> Tree<T> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            children: Vec::new(),
        }
    }

    pub fn get(&self) -> &T {
        &self.value
    }

    pub fn add_child_value(&mut self, value: T) {
        self.add_child(Self::new(value));
    }

    pub fn add_child(&mut self, child: Self) {
        self.children.push(child);
    }

    pub fn get_children(&self) -> &Vec<Tree<T>> {
        &self.children
    }

    pub fn find_mut<P>(&mut self, predicate: &P) -> Option<&mut Tree<T>>
    where
        P: Fn(&T) -> bool,
    {
        if predicate(&self.value) {
            return Some(self);
        }

        for child in self.children.iter_mut() {
            if let Some(t) = child.find_mut(predicate) {
                return Some(t);
            }
        }

        None
    }

    pub fn insert_below<P>(&mut self, predicate: &P, value: T) -> crate::error::Result<()>
    where
        P: Fn(&T) -> bool,
    {
        if let Some(p) = self.find_mut(predicate) {
            p.add_child_value(value);
            Ok(())
        } else {
            Err(crate::error::Error::new(
                "Couldn't find parent to insert below",
            ))
        }
    }
}

pub struct Forest<T>(Vec<Tree<T>>);

impl<T> Forest<T> {
    pub fn new() -> Self {
        Forest(Vec::new())
    }

    pub fn insert_below<P>(&mut self, predicate: &P, value: T)
    where
        P: Fn(&T) -> bool,
    {
        for tree in self.0.iter_mut() {
            if let Some(parent) = tree.find_mut(predicate) {
                parent.add_child_value(value);
                return;
            }
        }

        self.0.push(crate::tree::Tree::new(value));
    }

    pub fn trees(&self) -> &Vec<Tree<T>> {
        &self.0
    }

    pub fn into_trees(self) -> Vec<Tree<T>> {
        self.0
    }
}

pub struct TreeIterator<T> {
    value: Option<T>,
    children: Vec<TreeIterator<T>>,
}

impl<T> Iterator for TreeIterator<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(v) = self.value.take() {
            return Some(v);
        }

        if let Some(child) = self.children.last_mut() {
            if let Some(v) = child.next() {
                return Some(v);
            }

            self.children.pop();
            return self.next();
        }

        None
    }
}

impl<T> IntoIterator for Tree<T> {
    type Item = T;
    type IntoIter = TreeIterator<T>;

    fn into_iter(self) -> Self::IntoIter {
        TreeIterator {
            value: Some(self.value),
            children: self.children.into_iter().map(|t| t.into_iter()).collect(),
        }
    }
}

pub struct ForestIterator<T> {
    data: Vec<TreeIterator<T>>,
}

impl<T> Iterator for ForestIterator<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(child) = self.data.last_mut() {
            if let Some(v) = child.next() {
                return Some(v);
            }

            self.data.pop();
            return self.next();
        }

        None
    }
}

impl<T> IntoIterator for Forest<T> {
    type Item = T;
    type IntoIter = ForestIterator<T>;

    fn into_iter(self) -> Self::IntoIter {
        ForestIterator {
            data: self.0.into_iter().map(|t| t.into_iter()).collect(),
        }
    }
}
