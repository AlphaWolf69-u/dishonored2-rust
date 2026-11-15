//! Music playlist stack (141157080, 14115b650, 14115ba60, 14115c270).
use crate::assets::wwise::{MusicPlaylistItem, SelectionMode};
use super::super::containers::{Selection, Loops};
use std::{collections::HashMap, sync::{Arc, Mutex}};

pub type History = Arc<Mutex<HashMap<(u32, u32), Selection>>>;
#[derive(Debug)]
pub struct Playlist {
    pub items: Vec<MusicPlaylistItem>,
    children: Vec<Vec<usize>>,
    weights: Vec<Vec<u32>>,
}
impl Playlist {
    pub fn new(items: Vec<MusicPlaylistItem>) -> Result<Self, String> {
        let mut children = vec![Vec::new(); items.len()];
        fn walk(items: &[MusicPlaylistItem], children: &mut [Vec<usize>], index: usize) -> Result<usize, String> {
            let item = items.get(index).ok_or("music playlist child outside definition")?;
            let mut next = index + 1;
            for _ in 0..item.child_count {
                children[index].push(next);
                next = walk(items, children, next)?;
            }
            Ok(next)
        }
        if !items.is_empty() && walk(&items, &mut children, 0)? != items.len() {
            return Err("music playlist has trailing disconnected items".into());
        }
        let weights = children.iter().enumerate().map(|(index, children)| children.iter().map(|&child|
            if items[index].use_weights { items[child].weight } else { 50000 }).collect()).collect();
        Ok(Self { items, children, weights })
    }
}
struct Frame { index: usize, selection: Selection, loops: Loops, entered: bool }
impl Frame {
    fn new(index: usize, playlist: &Playlist, random15: &mut impl FnMut() -> u16) -> Self {
        let item = &playlist.items[index];
        Self { index, selection: Selection::default(), loops: Loops::music(item.loops, item.loop_min, item.loop_max, random15), entered: false }
    }
}
pub struct Cursor { stack: Vec<Frame>, done: bool }
impl Cursor {
    /// 14115bda0 walks playlist item IDs, reconstructs their ancestor stack
    /// and positions each parent's selector before returning the target leaf.
    pub fn jump(&mut self, actor: u32, playlist: &Playlist, id: u32, history: &History, mut random15: impl FnMut() -> u16) -> bool {
        fn find(playlist: &Playlist, index: usize, id: u32, path: &mut Vec<usize>) -> bool {
            path.push(index);
            if playlist.items[index].id == id { return true; }
            for &child in &playlist.children[index] { if find(playlist, child, id, path) { return true; } }
            path.pop();
            false
        }
        let mut path = Vec::new();
        if playlist.items.is_empty() || !find(playlist, 0, id, &mut path) { return false; }
        self.stack.clear();
        self.done = false;
        for (position, &index) in path.iter().enumerate() {
            let mut frame = Frame::new(index, playlist, &mut random15);
            if let Some(&child) = path.get(position + 1) {
                let selected = playlist.children[index].iter().position(|&index| index == child).unwrap();
                let item = &playlist.items[index];
                if matches!(item.kind, 1 | 3) {
                    history.lock().unwrap_or_else(|error| error.into_inner()).entry((actor, item.id)).or_default().seek_music(selected, item.avoid_repeat);
                } else { frame.selection.seek_music(selected, item.avoid_repeat); }
                frame.entered = true;
            }
            self.stack.push(frame);
        }
        true
    }
    pub fn new(playlist: &Playlist, mut random15: impl FnMut() -> u16) -> Self {
        Self { stack: if playlist.items.is_empty() { Vec::new() } else { vec![Frame::new(0, playlist, &mut random15)] }, done: false }
    }
    pub fn next(&mut self, actor: u32, playlist: &Playlist, history: &History, mut random15: impl FnMut() -> u16) -> Option<(u32, u32)> {
        if self.done { return None; }
        while let Some(frame) = self.stack.last_mut() {
            let item = &playlist.items[frame.index];
            if item.segment != 0 {
                if !frame.entered || frame.loops.next_cycle() {
                    frame.entered = true;
                    return Some((item.segment, item.id));
                }
                self.stack.pop();
                continue;
            }
            let step = matches!(item.kind, 1 | 3);
            if step && frame.entered { self.stack.pop(); continue; }
            let mode = if item.kind < 2 { SelectionMode::Sequence }
                else if item.shuffle { SelectionMode::Shuffle } else { SelectionMode::Random };
            let selected = if step {
                let mut history = history.lock().unwrap_or_else(|error| error.into_inner());
                history.entry((actor, item.id)).or_default().next_music(mode, &playlist.weights[frame.index], item.avoid_repeat, None, item.use_weights, &mut random15)
            } else {
                frame.selection.next_music(mode, &playlist.weights[frame.index], item.avoid_repeat, Some(&mut frame.loops), item.use_weights, &mut random15)
            };
            frame.entered = true;
            let Some(child) = selected.and_then(|selected| playlist.children[frame.index].get(selected).copied()) else {
                self.stack.pop(); continue;
            };
            self.stack.push(Frame::new(child, playlist, &mut random15));
        }
        self.done = true;
        None
    }
}
