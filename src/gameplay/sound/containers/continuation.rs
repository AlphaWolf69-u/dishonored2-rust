//! Shared continuation stacks and parallel-branch ownership
//! (14111ba70 / 14111bc60 / 14111bd70 / 14111bea0 / 141116b50).
use std::{collections::HashMap, sync::{Arc, Mutex}};
use crate::assets::wwise::TransitionMode;

pub enum Entry<T> {
    Sequence(T),
    Group(Branch<T>),
}

struct StackData<T> { entries: Mutex<Vec<Entry<T>>> }
impl<T> Drop for StackData<T> {
    fn drop(&mut self) {
        // The stack destructor releases every parallel-group count before
        // destroying its selection/actor references (141116b50 / 141116aa0).
        let entries = self.entries.get_mut().unwrap_or_else(|error| error.into_inner());
        for entry in entries.iter_mut() {
            if let Entry::Group(branch) = entry { branch.cancel(); }
        }
    }
}

/// Clones retain the same mutable stack, matching 141116a20. They do not
/// duplicate its entries or increment any parallel-branch count.
pub struct Stack<T>(Arc<StackData<T>>);
impl<T> Clone for Stack<T> { fn clone(&self) -> Self { Self(self.0.clone()) } }
impl<T> Default for Stack<T> {
    fn default() -> Self { Self(Arc::new(StackData { entries: Mutex::new(Vec::new()) })) }
}
impl<T> Stack<T> {
    pub fn push(&self, entry: Entry<T>) {
        self.0.entries.lock().unwrap_or_else(|error| error.into_inner()).push(entry);
    }
    pub fn pop(&self) -> Option<Entry<T>> {
        self.0.entries.lock().unwrap_or_else(|error| error.into_inner()).pop()
    }
    pub fn is_empty(&self) -> bool {
        self.0.entries.lock().unwrap_or_else(|error| error.into_inner()).is_empty()
    }
    pub fn with_last<R>(&self, f: impl FnOnce(Option<&mut Entry<T>>) -> R) -> R {
        f(self.0.entries.lock().unwrap_or_else(|error| error.into_inner()).last_mut())
    }
    fn identity(&self) -> usize { Arc::as_ptr(&self.0) as usize }

    /// 14112f0a0 follows the outer stacks of parallel entries to find the
    /// deepest sequence. Early group release is allowed only for TriggerRate.
    pub fn outer_sequence<R>(&self, mut f: impl FnMut(&T) -> R) -> Option<R> {
        let mut stack = self.clone();
        loop {
            let outer = {
                let entries = stack.0.entries.lock().unwrap_or_else(|error| error.into_inner());
                match entries.last()? {
                    Entry::Sequence(value) => return Some(f(value)),
                    Entry::Group(branch) => branch.outer()?,
                }
            };
            stack = outer;
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key { owner: u32, outer: usize }
struct Group<T> { outer: Stack<T>, remaining: u32, resume: bool }
type Groups<T> = Mutex<HashMap<Key, Group<T>>>;

/// The original actor's group map is keyed by the outer stack pointer.
/// Re-entering that actor with the same stack joins the existing group.
pub struct Registry<T>(Arc<Groups<T>>);
impl<T> Clone for Registry<T> { fn clone(&self) -> Self { Self(self.0.clone()) } }
impl<T> Default for Registry<T> { fn default() -> Self { Self(Arc::new(Mutex::new(HashMap::new()))) } }
impl<T> Registry<T> {
    /// `resume` is the inverse of the actor's virtual +0x2c8 result. Starting
    /// a group holds a sentinel until all child Play calls have returned.
    pub fn begin(&self, owner: u32, outer: Stack<T>, resume: bool) -> Session<T> {
        let key = Key { owner, outer: outer.identity() };
        {
            let mut groups = self.0.lock().unwrap_or_else(|error| error.into_inner());
            let group = groups.entry(key).or_insert(Group { outer, remaining: 0, resume });
            group.remaining = group.remaining.wrapping_add(1);
        }
        Session { sentinel: Branch { groups: self.0.clone(), key: Some(key) } }
    }
}

/// The initial sentinel is released without advancing a continuation
/// (141134aa0). It keeps synchronous failed child starts from ending a group
/// before the remaining authored children have been visited.
pub struct Session<T> { sentinel: Branch<T> }
impl<T> Session<T> {
    pub fn branch(&self) -> Branch<T> {
        let key = self.sentinel.key.expect("live group session");
        let mut groups = self.sentinel.groups.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(group) = groups.get_mut(&key) { group.remaining = group.remaining.wrapping_add(1); }
        Branch { groups: self.sentinel.groups.clone(), key: Some(key) }
    }
}

/// A branch belongs to one stack entry, not one source voice. Normal group
/// completion can return its outer stack. Dropping a stopped branch only
/// releases its count; it must never start the enclosing sequence.
pub struct Branch<T> { groups: Arc<Groups<T>>, key: Option<Key> }
impl<T> Branch<T> {
    pub fn outer(&self) -> Option<Stack<T>> {
        let groups = self.groups.lock().unwrap_or_else(|error| error.into_inner());
        groups.get(&self.key?).map(|group| group.outer.clone())
    }

    /// 14111bd70: only the final normal branch returns the outer continuation.
    pub fn finish(mut self) -> Option<Stack<T>> { self.release(true) }

    fn cancel(&mut self) { drop(self.release(false)); }
    fn release(&mut self, advance: bool) -> Option<Stack<T>> {
        let key = self.key.take()?;
        let completed = {
            let mut groups = self.groups.lock().unwrap_or_else(|error| error.into_inner());
            let group = groups.get_mut(&key)?;
            if group.remaining >= 2 { group.remaining -= 1; return None; }
            groups.remove(&key)
        }?;
        // Release the registry lock before dropping outer stacks, which may
        // themselves contain parallel entries using this registry.
        if advance && completed.resume { Some(completed.outer) } else { None }
    }
}
impl<T> Drop for Branch<T> { fn drop(&mut self) { self.cancel(); } }

pub enum Advance<N> {
    Ready(N),
    /// A parallel branch must wait for source end before releasing its group.
    AtSourceEnd,
    Exhausted,
}

/// 14112f0a0 advances shared sequence entries until it finds the next actor.
/// `select` includes the global actor lookup: a missing actor exhausts that
/// sequence entry, unlike an actor whose subsequent Play fails to make a PBI.
/// The returned candidate belongs to the still-retained stack; the pending
/// continuation action and the previous source may both reference that stack.
pub fn advance<T, N>(stack: &mut Option<Stack<T>>, early: bool,
    mut mode: impl FnMut(&T) -> TransitionMode,
    mut select: impl FnMut(&mut T) -> Option<N>) -> Advance<N> {
    enum Step<N> { Ready(N), SequenceEnded, Group, Empty }
    loop {
        let Some(current) = stack.as_ref() else { return Advance::Exhausted; };
        let step = current.with_last(|entry| match entry {
            Some(Entry::Sequence(sequence)) => match select(sequence) {
                Some(next) => Step::Ready(next), None => Step::SequenceEnded,
            },
            Some(Entry::Group(_)) => Step::Group,
            None => Step::Empty,
        });
        match step {
            Step::Ready(next) => return Advance::Ready(next),
            Step::Empty => { *stack = None; return Advance::Exhausted; },
            Step::SequenceEnded => { drop(current.pop()); },
            Step::Group => {
                if early && current.outer_sequence(&mut mode) != Some(TransitionMode::TriggerRate) {
                    return Advance::AtSourceEnd;
                }
                let outer = match current.pop() {
                    Some(Entry::Group(branch)) => branch.finish(),
                    _ => None,
                };
                *stack = outer;
                if stack.is_none() { return Advance::Exhausted; }
            },
        }
    }
}
