//!5d6b70/5d7050/5d79d0/5d8190 Crew tree instance lifetime and the actual
//!5d83c0/5d8440 stack interpreter for admitted Loop/Parallel/Idle/DebugLog.
use crate::{
    assets::ai_world::crew_tree::{Definition, Kind},
    gameplay::entity_lifetime::EntityId,
};
use std::sync::Arc;

#[derive(Clone, Copy)]
struct NodeState {
    last: u8,
    second: u8,
    waiting: bool,
    loop_push: bool,
    remaining: i32,
    stack: usize,
    branch: Option<usize>,
    debug_seen: bool,
    debug_until: f64,
}
impl Default for NodeState {
    fn default() -> Self {
        Self {
            last: 2,
            second: 2,
            waiting: false,
            loop_push: false,
            remaining: 0,
            stack: 0,
            branch: None,
            debug_seen: false,
            debug_until: -1.,
        }
    }
}
struct Stack {
    nodes: Vec<usize>,
    owner: Option<usize>,
}
enum Decision {
    Yield,
    Done(u8),
    Push(usize),
    Fork(usize, usize),
}

///5d6fd0 stores the actual actor generation and CrewCore query result.
///No NPC/perception record is created by these four node contracts.
#[derive(Clone, Copy)]
struct ActorContext {
    entity: EntityId,
    has_ai_core: bool,
}
pub struct Instance {
    definition: Arc<Definition>,
    actor: ActorContext,
    context: Option<ActorContext>,
    states: Box<[NodeState]>,
    stacks: Box<[Stack]>,
    root_result: u8,
    stimuli: Vec<u32>,
}
impl Instance {
    pub fn new(definition: Arc<Definition>, entity: EntityId, has_ai_core: bool) -> Self {
        let data = definition.data();
        let states = vec![NodeState::default(); data.nodes.len()].into_boxed_slice();
        let stacks = (0..=data.parallel_count)
            .map(|_| Stack {
                nodes: Vec::with_capacity(data.maximum_depth),
                owner: None,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            definition,
            actor: ActorContext {
                entity,
                has_ai_core,
            },
            context: None,
            states,
            stacks,
            root_result: 2,
            //5d6b70: +48 list header, +58..+77 eight inline u32 slots.
            stimuli: Vec::with_capacity(8),
        }
    }
    ///588310→5d6fd0/5d7050 instantiates and resets. Execution starts only
    ///when the actual Decision Update588950 calls5d89f0.
    pub fn start(&mut self) -> Result<(), String> {
        self.reset();
        self.context = Some(self.actor);
        Ok(())
    }
    ///5d79d0 aborts every active task and clears stacks/continuations. The
    ///admitted node Stop hooks are no-ops except Loop's state reset5c1150.
    pub fn reset(&mut self) {
        for stack in &mut self.stacks {
            stack.nodes.clear();
            stack.owner = None;
        }
        self.states.fill(NodeState::default());
        self.root_result = 2;
        self.stimuli.clear();
    }
    ///5883f0→5d8190 resets then destroys the instance context. The component
    ///drops this instance afterward, releasing all storage and its tree lease.
    pub fn stop(&mut self) {
        self.reset();
        self.context = None;
    }
    pub fn entity(&self) -> EntityId {
        self.actor.entity
    }
    pub fn definition(&self) -> &Arc<Definition> {
        &self.definition
    }
    ///LAB588af0→5d7830 inserts a stimulus by unsigned sorted identity,
    ///deduplicating the actual queued mask even when no task handles it.
    pub fn stimulus(&mut self, mask: u32) -> Result<(), String> {
        if self.context.is_none() {
            return Err("Crew arktree stimulus before Start".into());
        }
        if let Err(index) = self.stimuli.binary_search(&mask) {
            self.stimuli.insert(index, mask);
        }
        Ok(())
    }
    ///588560→5d7c30 revisits only live stack tasks.5c10c0's optional NPC
    ///binding is not consumed by these admitted four task classes; their
    ///actual virtual+18 hooks reset Loop counts and DebugLog deadlines.
    pub fn pause(&mut self) {
        for stack in &self.stacks {
            for &node in &stack.nodes {
                match self.definition.data().nodes[node].kind {
                    Kind::Loop(count) if count > 0 => self.states[node].remaining = count,
                    Kind::DebugLog { .. } => {
                        self.states[node].debug_seen = false;
                        self.states[node].debug_until = -1.;
                    }
                    _ => {}
                }
            }
        }
    }
    pub fn has_ai_core(&self) -> bool {
        self.actor.has_ai_core
    }

    fn decision(&mut self, node: usize, entity_time: f64) -> Decision {
        match self.definition.data().nodes[node].kind {
            Kind::Idle => Decision::Yield, //5a4df0: task result2.
            Kind::DebugLog {
                output_to_console,
                return_immediately,
                display_time,
            } => {
                let state = &mut self.states[node];
                //5d3110/5d3130. Release code consumes no text/colour and
                //does not print merely because outputToConsole is true.
                if !state.debug_seen {
                    state.debug_seen = true;
                    if return_immediately {
                        return Decision::Done(1);
                    }
                    if display_time > 0. {
                        state.debug_until = entity_time + f64::from(display_time);
                    } else if display_time >= 0. && !output_to_console {
                        return Decision::Done(1);
                    }
                    Decision::Yield
                } else if display_time <= 0. || entity_time <= state.debug_until {
                    Decision::Yield
                } else {
                    Decision::Done(1)
                }
            }
            Kind::Loop(condition) => {
                //5c1160 initializes stop policy;5c15b0 yields between
                //repeated children instead of spinning a complete child.
                let state = &mut self.states[node];
                if state.last == 2 {
                    state.loop_push = true;
                } else if (condition == 0 && state.last == 0) || (condition < 0 && state.last == 1)
                {
                    return Decision::Done(state.last);
                } else if condition > 0 {
                    state.remaining -= 1;
                    if state.remaining == 0 {
                        return Decision::Done(state.last);
                    }
                }
                if state.loop_push {
                    state.loop_push = false;
                    Decision::Push(self.definition.data().nodes[node].children[0])
                } else {
                    state.loop_push = true;
                    Decision::Yield
                }
            }
            Kind::Parallel {
                first_fail,
                first_success,
            } => {
                let state = self.states[node];
                if state.last == 2 {
                    let children = &self.definition.data().nodes[node].children;
                    Decision::Fork(children[1], children[0])
                } else if state.second != 2 {
                    Decision::Done(state.second)
                } else if (first_fail && state.last == 0) || (first_success && state.last == 1) {
                    Decision::Done(state.last)
                } else {
                    Decision::Yield
                }
            }
        }
    }
    fn push(&mut self, stack: usize, node: usize) {
        self.states[node] = NodeState {
            stack,
            remaining: match self.definition.data().nodes[node].kind {
                Kind::Loop(count) => count,
                _ => 0,
            },
            ..Default::default()
        };
        self.stacks[stack].nodes.push(node);
    }
    ///5c1100/5c1110 deliver real child results to the parent continuation.
    fn child_finished(&mut self, parent: usize, result: u8) {
        let state = &mut self.states[parent];
        if let Kind::Parallel {
            first_fail,
            first_success,
        } = self.definition.data().nodes[parent].kind
        {
            if state.last == 2 {
                state.last = result;
                if (first_fail && result == 0) || (first_success && result == 1) {
                    state.waiting = false;
                }
            } else if state.waiting {
                state.second = result;
                state.waiting = false;
            }
        } else {
            state.last = result;
            state.waiting = false;
        }
    }
    fn abort_above(&mut self, stack: usize, parent: Option<usize>) {
        while let Some(&node) = self.stacks[stack].nodes.last() {
            if Some(node) == parent {
                break;
            }
            self.stacks[stack].nodes.pop();
            if let Some(branch) = self.states[node].branch.take() {
                self.abort_above(branch, None);
                self.stacks[branch].owner = None;
            }
            self.states[node] = NodeState::default();
        }
    }
    fn run_stack(&mut self, stack: usize, entity_time: f64) -> Result<(), String> {
        //5d8440's actual infinite-continuation diagnostic, not a frame budget.
        for _ in 0..1000 {
            let Some(&node) = self.stacks[stack].nodes.last() else {
                return Ok(());
            };
            if self.states[node].waiting {
                return Ok(());
            }
            match self.decision(node, entity_time) {
                Decision::Yield => return Ok(()),
                Decision::Push(child) => {
                    self.states[node].waiting = true;
                    self.push(stack, child);
                }
                Decision::Fork(last, first) => {
                    let branch = (stack + 1..self.stacks.len())
                        .find(|&index| self.stacks[index].owner.is_none())
                        .ok_or("arktree parallel continuation capacity differs from definition")?;
                    self.stacks[branch].owner = Some(node);
                    self.states[node].branch = Some(branch);
                    self.states[node].waiting = true;
                    self.push(stack, last);
                    self.push(branch, first);
                }
                Decision::Done(result) => {
                    self.stacks[stack].nodes.pop();
                    if let Some(branch) = self.states[node].branch.take() {
                        self.abort_above(branch, None);
                        self.stacks[branch].owner = None;
                    }
                    if let Some(&parent) = self.stacks[stack].nodes.last() {
                        self.child_finished(parent, result);
                    } else if let Some(parent) = self.stacks[stack].owner {
                        self.child_finished(parent, result);
                        if !self.states[parent].waiting {
                            self.abort_above(self.states[parent].stack, Some(parent));
                        }
                        return Ok(());
                    } else {
                        self.root_result = result;
                        return Ok(());
                    }
                }
            }
        }
        Err("Crew arktree traversal infinite loop (1000 continuations)".into())
    }
    ///5d89f0→5d83c0 updates main then occupied auxiliary stacks. Return3
    ///means suspended/running at interpreter level;0/1 are completed results.
    ///DebugLog receives actual8778030 entity time from the canonical clock.
    pub fn update(&mut self, entity_time: f64) -> Result<u8, String> {
        if self.context.is_none() {
            return Err("Crew arktree update before Start".into());
        }
        //5d7530 scans the actual handled/consumed masks before traversal,
        //then clears the accepted queue. The declaration owner admits only
        //zero masks, so every source match is false and no task is restarted.
        self.stimuli.clear();
        if self.stacks[0].nodes.is_empty() && self.root_result == 2 {
            self.push(0, 0);
        }
        self.run_stack(0, entity_time)?;
        for stack in 1..self.stacks.len() {
            if self.stacks[stack].owner.is_some() {
                self.run_stack(stack, entity_time)?;
            }
        }
        Ok(if self.root_result == 2 {
            3
        } else {
            self.root_result
        })
    }
}
