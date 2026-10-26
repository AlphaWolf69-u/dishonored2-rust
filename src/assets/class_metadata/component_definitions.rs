//! Constant declarations made by the installed component static constructors.
//! This is a bounded reader for their literal MOV/LEA instruction grammar, not
//! a foreign-code runtime. Allocation/list operations become owned Rust data.
use super::{ComponentTypeDeclaration, Image};
use iced_x86::{Decoder, DecoderOptions, Instruction, Mnemonic, OpKind, Register};
use std::collections::HashMap;

#[derive(Debug)]
pub struct ComponentDependencyDeclaration {
    pub class: usize,
    pub reason: String,
    pub exact: bool,
}

/// The two additional source declaration lists retain their integer argument
/// independently of initialization/update dependency flags.
#[derive(Debug)]
pub struct ComponentOrderingDeclaration {
    pub class: usize,
    pub argument: u64,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct ComponentDefinition {
    pub ordering: [Vec<ComponentOrderingDeclaration>; 2],
    pub initialization: Vec<ComponentDependencyDeclaration>,
    pub updates: [Vec<ComponentDependencyDeclaration>; 8],
    pub stage_values: [u32; 8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Value {
    Unknown,
    Scalar(u64),
    Partial { bits: u64, mask: u64 },
    Image(u32),
    Stack(i64),
}
impl Value {
    fn offset(self, delta: i64) -> Result<Self, String> {
        match self {
            Self::Scalar(v) => Ok(Self::Scalar(v.wrapping_add(delta as u64))),
            Self::Image(v) => Ok(Self::Image(
                u32::try_from(i64::from(v) + delta)
                    .map_err(|_| "declaration image address overflow")?,
            )),
            Self::Stack(v) => Ok(Self::Stack(
                v.checked_add(delta).ok_or("declaration stack overflow")?,
            )),
            Self::Unknown | Self::Partial { .. } => {
                Err("declaration address uses an undefined value".into())
            }
        }
    }
    fn scalar(self) -> Result<u64, String> {
        if let Self::Scalar(v) = self {
            Ok(v)
        } else {
            Err("declaration argument is not a scalar".into())
        }
    }
    fn image(self) -> Result<u32, String> {
        if let Self::Image(v) = self {
            Ok(v)
        } else {
            Err("declaration argument is not an image address".into())
        }
    }
    fn byte(self) -> Result<u64, String> {
        match self {
            Self::Scalar(v) => Ok(v & 255),
            Self::Partial { bits, mask } if mask & 255 == 255 => Ok(bits & 255),
            _ => Err("undefined declaration byte argument".into()),
        }
    }
}

struct Reader<'a, 'b> {
    image: &'a Image<'b>,
    classes: &'a HashMap<u32, usize>,
    definition: u32,
    output: ComponentDefinition,
    registers: [Value; 16],
    stack: HashMap<i64, (usize, Value)>,
    instructions: usize,
    lists_initialized: [bool; 11],
}
impl Reader<'_, '_> {
    fn register_slot(reg: Register) -> Result<usize, String> {
        let full = reg.full_register();
        if !(Register::RAX..=Register::R15).contains(&full) {
            return Err("declaration uses a non-integer register".into());
        }
        Ok(full.number())
    }
    fn register(&self, reg: Register) -> Result<Value, String> {
        let value = self.registers[Self::register_slot(reg)?];
        if reg.size() == 8 || value == Value::Unknown {
            return Ok(value);
        }
        if matches!(
            reg,
            Register::AH | Register::BH | Register::CH | Register::DH
        ) {
            return Err("unsupported high-byte declaration register".into());
        }
        match value {
            Value::Scalar(v) => Ok(Value::Scalar(v & (u64::MAX >> ((8 - reg.size()) * 8)))),
            Value::Partial { bits, mask }
                if mask & (u64::MAX >> ((8 - reg.size()) * 8))
                    == (u64::MAX >> ((8 - reg.size()) * 8)) =>
            {
                Ok(Value::Scalar(bits & (u64::MAX >> ((8 - reg.size()) * 8))))
            }
            _ => Err("declaration truncates a symbolic address".into()),
        }
    }
    fn set_register(&mut self, reg: Register, value: Value) -> Result<(), String> {
        let slot = Self::register_slot(reg)?;
        if matches!(
            reg,
            Register::AH | Register::BH | Register::CH | Register::DH
        ) {
            return Err("unsupported high-byte declaration register".into());
        }
        self.registers[slot] = match reg.size() {
            8 => value,
            4 => Value::Scalar(value.scalar()? & 0xffff_ffff),
            size @ (1 | 2) => {
                let low_mask = u64::MAX >> ((8 - size) * 8);
                let (bits, mask) = match self.registers[slot] {
                    Value::Scalar(bits) => (bits, u64::MAX),
                    Value::Partial { bits, mask } => (bits, mask),
                    Value::Unknown => (0, 0),
                    _ => return Err("declaration partially overwrites an address".into()),
                };
                let bits = bits & !low_mask | value.scalar()? & low_mask;
                let mask = mask | low_mask;
                if mask == u64::MAX {
                    Value::Scalar(bits)
                } else {
                    Value::Partial { bits, mask }
                }
            }
            _ => return Err("unsupported declaration register width".into()),
        };
        Ok(())
    }
    fn address(&self, instruction: &Instruction) -> Result<Value, String> {
        if instruction.is_ip_rel_memory_operand() {
            let rva = instruction
                .ip_rel_memory_address()
                .checked_sub(self.image.base)
                .and_then(|v| u32::try_from(v).ok())
                .ok_or("declaration RIP address exceeds image")?;
            return Ok(Value::Image(rva));
        }
        if instruction.memory_index() != Register::None {
            return Err("indexed declaration operand is unsupported".into());
        }
        self.register(instruction.memory_base())?
            .offset(instruction.memory_displacement64() as i64)
    }
    fn read_stack(&self, address: Value, size: usize) -> Result<Value, String> {
        let Value::Stack(at) = address else {
            return Err("declaration reads non-stack memory".into());
        };
        if let Some(&(stored, value)) = self.stack.get(&at) {
            if stored == size || value == Value::Unknown {
                return Ok(value);
            }
            if stored > size {
                return Ok(Value::Scalar(
                    value.scalar()? & (u64::MAX >> ((8 - size) * 8)),
                ));
            }
        }
        Err(format!(
            "declaration reads undefined stack data at {at} ({size} bytes)"
        ))
    }
    fn operand(&self, i: &Instruction, op: u32) -> Result<Value, String> {
        match i.op_kind(op) {
            OpKind::Register => self.register(i.op_register(op)),
            OpKind::Memory => self.read_stack(self.address(i)?, i.memory_size().size()),
            _ => i
                .try_immediate(op)
                .map(Value::Scalar)
                .map_err(|_| "unsupported declaration operand".into()),
        }
    }
    fn store(&mut self, address: Value, size: usize, value: Value) -> Result<(), String> {
        match address {
            Value::Stack(at) => {
                self.stack.insert(at, (size, value));
                Ok(())
            }
            Value::Image(rva) => {
                let offset = rva
                    .checked_sub(self.definition)
                    .ok_or("declaration writes unrelated global")?;
                if offset < 0xb8
                    || offset as usize + size > 0xd8
                    || offset % 4 != 0
                    || !matches!(size, 4 | 8)
                {
                    return Err("declaration writes an unsupported definition field".into());
                }
                let scalar = value.scalar()?;
                for word in 0..size / 4 {
                    self.output.stage_values[(offset as usize - 0xb8) / 4 + word] =
                        (scalar >> (word * 32)) as u32;
                }
                Ok(())
            }
            _ => Err("declaration store is not owned stack/definition data".into()),
        }
    }
    fn text(&self, value: Value) -> Result<String, String> {
        String::from_utf8(self.image.string(value.image()?)?)
            .map_err(|_| "non-UTF8 component dependency reason".into())
    }
    fn class(&self, value: Value) -> Result<usize, String> {
        self.classes
            .get(&value.image()?)
            .copied()
            .ok_or_else(|| "component dependency names an unknown class".into())
    }
    fn dependency(
        &mut self,
        stage: Option<usize>,
        class: Value,
        reason: Value,
        exact: Value,
    ) -> Result<(), String> {
        let exact = exact.byte()?;
        if exact > 1 {
            return Err("invalid exact-class dependency flag".into());
        }
        let record = ComponentDependencyDeclaration {
            class: self.class(class)?,
            reason: self.text(reason)?,
            exact: exact != 0,
        };
        if let Some(stage) = stage {
            self.output
                .updates
                .get_mut(stage)
                .ok_or("dependency stage exceeds eight stages")?
                .push(record);
        } else {
            self.output.initialization.push(record);
        }
        Ok(())
    }
    fn primitive(&mut self, rva: u32) -> Result<bool, String> {
        let a = self.register(Register::RCX)?;
        let b = self.register(Register::RDX)?;
        let c = self.register(Register::R8)?;
        let d = self.register(Register::R9)?;
        match rva {
            0x998b0 => {} // Audited empty declaration function: RET.
            0x500c30 => {
                let offset = a
                    .image()?
                    .checked_sub(self.definition)
                    .ok_or("list belongs to another definition")?;
                if !matches!(offset, 8 | 0x18 | 0x28) {
                    return Err("unexpected definition list constructor".into());
                }
                self.lists_initialized[((offset - 8) / 16) as usize] = true;
            }
            0x11ad700 => {
                if a != Value::Image(self.definition + 0x38)
                    || b != Value::Scalar(0x10)
                    || c != Value::Scalar(8)
                    || d != Value::Image(0x34ef70)
                    || self.read_stack(self.register(Register::RSP)?.offset(0x28)?, 8)?
                        != Value::Image(0x62140)
                {
                    return Err("unsupported definition array constructor".into());
                }
                self.lists_initialized[3..].fill(true);
            }
            0x706f30 => {
                if a != Value::Image(self.definition) {
                    return Err("initialization dependency targets another definition".into());
                }
                self.dependency(None, b, c, d)?;
            }
            0x706f60 => {
                if a != Value::Image(self.definition) {
                    return Err("update dependency targets another definition".into());
                }
                let exact = self.read_stack(self.register(Register::RSP)?.offset(0x28)?, 1)?;
                self.dependency(Some(c.scalar()? as usize), b, d, exact)?;
            }
            0x39de50 => {
                let offset = a
                    .image()?
                    .checked_sub(self.definition)
                    .ok_or("ordering targets another definition")?;
                if !matches!(offset, 8 | 0x18) {
                    return Err("unexpected ordering list".into());
                }
                let class = self.class(self.read_stack(b, 8)?)?;
                let argument = self.read_stack(b.offset(8)?, 8)?.scalar()?;
                let reason = self.text(self.read_stack(b.offset(16)?, 8)?)?;
                self.output.ordering[((offset - 8) / 16) as usize].push(
                    ComponentOrderingDeclaration {
                        class,
                        argument,
                        reason,
                    },
                );
            }
            0x707460 => {
                if a != Value::Image(self.definition) {
                    return Err("stage value targets another definition".into());
                }
                *self
                    .output
                    .stage_values
                    .get_mut(b.scalar()? as usize)
                    .ok_or("invalid declaration stage")? = c.scalar()? as u32;
            }
            0x11ac5bc => {
                if !self.image.executable_rva(a.image()?) {
                    return Err("invalid static destructor declaration".into());
                }
                // Source registers a process destructor. Rust owns and drops
                // these lists; no foreign destructor or CRT call is needed.
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn function(&mut self, rva: u32, depth: usize) -> Result<(), String> {
        if depth > 32 {
            return Err("component declaration call depth exceeded".into());
        }
        if self.primitive(rva)? {
            return Ok(());
        }
        let section = self
            .image
            .sections
            .iter()
            .find(|s| s.executable && s.contains(rva, 1))
            .ok_or("component declaration target is not code")?;
        let tail = section
            .raw(self.image.bytes)?
            .get((rva - section.rva) as usize..)
            .ok_or("declaration has no code")?;
        let mut decoder = Decoder::with_ip(
            64,
            tail,
            self.image.base + u64::from(rva),
            DecoderOptions::NONE,
        );
        let initial_rsp = self.register(Register::RSP)?;
        loop {
            self.instructions += 1;
            if self.instructions > 8192 {
                return Err("component declaration instruction budget exceeded".into());
            }
            let i = decoder.decode();
            let result = match i.mnemonic() {
                Mnemonic::Mov => {
                    let value = self.operand(&i, 1)?;
                    match i.op0_kind() {
                        OpKind::Register => self.set_register(i.op0_register(), value),
                        OpKind::Memory => {
                            self.store(self.address(&i)?, i.memory_size().size(), value)
                        }
                        _ => Err("unsupported declaration MOV destination".into()),
                    }
                }
                Mnemonic::Lea => {
                    let address = self.address(&i)?;
                    self.set_register(i.op0_register(), address)
                }
                Mnemonic::Xor
                    if i.op0_kind() == OpKind::Register && i.op0_register() == i.op1_register() =>
                {
                    self.set_register(i.op0_register(), Value::Scalar(0))
                }
                Mnemonic::Add | Mnemonic::Sub => {
                    let delta = self.operand(&i, 1)?.scalar()? as i64;
                    let delta = if i.mnemonic() == Mnemonic::Sub {
                        delta
                            .checked_neg()
                            .ok_or("declaration subtraction overflow")?
                    } else {
                        delta
                    };
                    self.set_register(
                        i.op0_register(),
                        self.register(i.op0_register())?.offset(delta)?,
                    )
                }
                Mnemonic::Push => {
                    let value = self.operand(&i, 0)?;
                    let address = self.register(Register::RSP)?.offset(-8)?;
                    self.set_register(Register::RSP, address)?;
                    self.store(address, 8, value)
                }
                Mnemonic::Pop => {
                    let rsp = self.register(Register::RSP)?;
                    let value = self.read_stack(rsp, 8)?;
                    self.set_register(i.op0_register(), value)?;
                    self.set_register(Register::RSP, rsp.offset(8)?)
                }
                Mnemonic::Call | Mnemonic::Jmp if i.op0_kind() == OpKind::NearBranch64 => {
                    let target = i
                        .near_branch_target()
                        .checked_sub(self.image.base)
                        .and_then(|v| u32::try_from(v).ok())
                        .ok_or("declaration branch exceeds image")?;
                    let rsp = self.register(Register::RSP)?;
                    let tail = i.mnemonic() == Mnemonic::Jmp;
                    if !tail {
                        self.set_register(Register::RSP, rsp.offset(-8)?)?;
                    }
                    self.function(target, depth + 1)?;
                    if tail {
                        if rsp != initial_rsp {
                            return Err("unbalanced declaration tail call".into());
                        }
                        return Ok(());
                    }
                    self.set_register(Register::RSP, rsp)?;
                    // Win64 caller-saved registers cannot supply subsequent
                    // constants unless the constructor defines them again.
                    for reg in [
                        Register::RAX,
                        Register::RCX,
                        Register::RDX,
                        Register::R8,
                        Register::R9,
                        Register::R10,
                        Register::R11,
                    ] {
                        self.registers[Self::register_slot(reg)?] = Value::Unknown;
                    }
                    Ok(())
                }
                Mnemonic::Nop => Ok(()),
                Mnemonic::Ret if self.register(Register::RSP)? == initial_rsp => return Ok(()),
                _ => Err(format!(
                    "unsupported constant declaration instruction {:?}",
                    i.mnemonic()
                )),
            };
            result.map_err(|e| format!("{e} at {:#x}", i.ip()))?;
        }
    }
}

pub(super) fn read(
    image: &Image<'_>,
    types: &[ComponentTypeDeclaration],
    classes: &[super::ClassDeclaration],
) -> Result<Vec<ComponentDefinition>, String> {
    let classes: HashMap<_, _> = classes
        .iter()
        .enumerate()
        .map(|(i, c)| (c.class_info_rva, i))
        .collect();
    let ranges = image.function_ranges()?;
    let definitions: HashMap<_, _> = types
        .iter()
        .enumerate()
        .map(|(i, t)| (t.definition_rva + 8, i))
        .collect();
    let mut constructors = vec![None; types.len()];
    for section in image.sections.iter().filter(|s| s.executable) {
        let code = section.raw(image.bytes)?;
        for at in 0..code.len().saturating_sub(6) {
            if code[at..at + 3] != [0x48, 0x8d, 0x0d] {
                continue;
            }
            let target = i64::from(section.rva)
                + at as i64
                + 7
                + i64::from(i32::from_le_bytes(code[at + 3..at + 7].try_into().unwrap()));
            let Some(&index) = u32::try_from(target).ok().and_then(|t| definitions.get(&t)) else {
                continue;
            };
            let rva = section.rva + at as u32;
            let pos = ranges.partition_point(|range| range.start <= rva);
            let Some(range) = pos
                .checked_sub(1)
                .and_then(|p| ranges.get(p))
                .filter(|r| r.contains(&rva))
            else {
                continue;
            };
            let mut decoder = Decoder::with_ip(
                64,
                image.read(range.start, (range.end - range.start) as usize)?,
                image.base + u64::from(range.start),
                DecoderOptions::NONE,
            );
            let mut calls = Vec::new();
            while decoder.can_decode() {
                let i = decoder.decode();
                if i.mnemonic() == Mnemonic::Call && i.op0_kind() == OpKind::NearBranch64 {
                    calls.push(i.near_branch_target());
                }
            }
            if calls.get(..3) != Some(&[image.base + 0x500c30; 3]) {
                continue;
            }
            if constructors[index].replace(range.start).is_some() {
                return Err("ambiguous component definition constructor".into());
            }
        }
    }
    types
        .iter()
        .zip(constructors)
        .map(|(entry, constructor)| {
            let constructor =
                constructor.ok_or("component definition has no constant constructor")?;
            let mut reader = Reader {
                image,
                classes: &classes,
                definition: entry.definition_rva,
                output: ComponentDefinition::default(),
                registers: [Value::Unknown; 16],
                stack: HashMap::new(),
                instructions: 0,
                lists_initialized: [false; 11],
            };
            reader.set_register(Register::RSP, Value::Stack(0))?;
            reader
                .function(constructor, 0)
                .map_err(|e| format!("component{}: {e}", entry.pool_index))?;
            if reader.lists_initialized.contains(&false) {
                return Err("component constructor left lists uninitialized".into());
            }
            Ok(reader.output)
        })
        .collect()
}
