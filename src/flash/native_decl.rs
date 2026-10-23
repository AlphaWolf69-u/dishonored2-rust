//! Types of the Iggy native-class table (`native_table.rs`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Method,
    Getter,
    Setter,
    StaticMethod,
    StaticGetter,
    StaticSetter,
    StaticConst,
    /// Native field read/write through a shared accessor.
    FieldGet,
    FieldSet,
    Var,
    StaticVar,
    /// A function installed on the class prototype (`FUN_141062580`).
    ProtoMethod,
    GlobalFunction,
    GlobalConst,
}

/// The namespace a member is registered in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ns {
    Public,
    /// `http://adobe.com/AS3/2006/builtin`
    As3,
    Uri(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Const {
    Int(i64),
    Number(f64),
    Str(&'static str),
    Bool(bool),
    Null,
    Undefined,
}

/// How Iggy implements a member.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Imp {
    /// A native function at this executable address.
    Native(u64),
    /// `LAB_1410b38e0`: throws IllegalOperationError.
    ThrowIllegalOperation,
    /// `LAB_1410b2930`: throws TypeError.
    ThrowType,
    /// `FUN_1400998b0`: returns without writing a result.
    Noop,
    Return(Const),
    /// `LAB_1410802b0`: returns the receiver.
    ReturnThis,
    /// An interface method declaration.
    Interface,
    None,
}

/// Registration conditions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    Always,
    /// Registered when the movie's SWF version is above 9.
    SwfVersionAbove9,
    /// Gated on the VM's light-VM flag (`VM+0x6b40`, from the
    /// arkDeclIggyFile `m_isLightVM` bit): present in full VMs.
    Unknown,
}

#[derive(Clone, Copy, Debug)]
pub struct Member {
    pub kind: Kind,
    pub name: &'static str,
    /// Package of a global function or constant.
    pub package: &'static str,
    pub ty: &'static str,
    pub ns: Ns,
    pub imp: Imp,
    pub value: Const,
    /// Native field offset for `FieldGet` / `FieldSet`.
    pub offset: u32,
    pub cond: Cond,
}

#[derive(Debug)]
pub struct ClassDecl {
    pub package: &'static str,
    pub name: &'static str,
    /// Simple name of the base class, empty for none.
    pub base: &'static str,
    pub ctor: Imp,
    pub interface: bool,
    pub interfaces: &'static [&'static str],
    pub cond: Cond,
    pub members: &'static [Member],
}

/// Declarations that are not classes: the global function holder and
/// VM-internal helpers.
pub fn is_pseudo(decl: &ClassDecl) -> bool {
    decl.name.starts_with('(') || decl.name == "___ScriptObject" || decl.name == "MethodClosure"
}

pub fn find(package: &str, name: &str) -> Option<&'static ClassDecl> {
    super::native_table::CLASSES.iter().find(|c| c.package == package && c.name == name)
}

pub fn globals() -> &'static ClassDecl {
    super::native_table::CLASSES.iter().find(|c| c.name.starts_with("(global")).expect("global function table")
}
