//! Inspect an installed Iggy movie: root, character kinds and its AS3 code.
//!
//! Usage: iggy-audit <resource> [--classes] [--ops] [--disassemble <class>]
use dishonored2_rust::assets::{archive::Catalog, iggy::IggyFile, installation};
use std::collections::BTreeMap;
use std::env;
use swf::avm2::read::Reader;
use swf::extensions::ReadSwfExt;
use swf::avm2::types::{AbcFile, Index, Multiname, Namespace, Op};

fn string(abc: &AbcFile, index: &Index<String>) -> String {
    match index.0 {
        0 => "*".into(),
        i => abc
            .constant_pool
            .strings
            .get(i as usize - 1)
            .map(|s| String::from_utf8_lossy(s).into_owned())
            .unwrap_or_else(|| format!("<string {i}>")),
    }
}

fn namespace(abc: &AbcFile, index: &Index<Namespace>) -> String {
    if index.0 == 0 {
        return "*".into();
    }
    match abc.constant_pool.namespaces.get(index.0 as usize - 1) {
        Some(
            Namespace::Namespace(s)
            | Namespace::Package(s)
            | Namespace::PackageInternal(s)
            | Namespace::Protected(s)
            | Namespace::Explicit(s)
            | Namespace::StaticProtected(s)
            | Namespace::Private(s),
        ) => string(abc, s),
        None => format!("<ns {}>", index.0),
    }
}

fn multiname(abc: &AbcFile, index: &Index<Multiname>) -> String {
    if index.0 == 0 {
        return "*".into();
    }
    match abc.constant_pool.multinames.get(index.0 as usize - 1) {
        Some(Multiname::QName { namespace: ns, name }) => {
            let ns = namespace(abc, ns);
            let name = string(abc, name);
            if ns.is_empty() { name } else { format!("{ns}:{name}") }
        }
        Some(Multiname::Multiname { name, .. } | Multiname::RTQName { name }) => string(abc, name),
        Some(Multiname::TypeName { base_type, parameters }) => {
            let params: Vec<_> = parameters.iter().map(|p| multiname(abc, p)).collect();
            format!("{}.<{}>", multiname(abc, base_type), params.join(","))
        }
        Some(other) => format!("{other:?}"),
        None => format!("<multiname {}>", index.0),
    }
}

fn op_name(op: &Op) -> String {
    let text = format!("{op:?}");
    text.split([' ', '{', '(']).next().unwrap_or_default().to_owned()
}

fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let resource = args.first().ok_or("usage: iggy-audit <resource> [--classes] [--ops] [--disassemble <class>]")?;
    let catalog = Catalog::open(&installation::game_base().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let bytes = catalog
        .read(resource)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{resource} is not indexed"))?;
    let file = IggyFile::parse(bytes).map_err(|e| e.to_string())?;
    let movie = file.movie().map_err(|e| e.to_string())?;
    println!("{movie:#?}");
    let mut kinds = BTreeMap::<Option<u8>, usize>::new();
    for i in 0..movie.character_count as usize {
        let kind = file.character(i).map_err(|e| e.to_string())?.map(|c| c.u8(0).unwrap());
        *kinds.entry(kind).or_default() += 1;
    }
    println!("character kinds (None = imported): {kinds:?}");
    let Some(abc) = file.abc().map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let abc = Reader::new(abc).read().map_err(|e| e.to_string())?;
    println!(
        "abc: {} classes, {} methods, {} bodies, {} scripts",
        abc.classes.len(),
        abc.methods.len(),
        abc.method_bodies.len(),
        abc.scripts.len()
    );
    if args.iter().any(|a| a == "--classes") {
        for instance in &abc.instances {
            println!(
                "class {} extends {}",
                multiname(&abc, &instance.name),
                multiname(&abc, &instance.super_name)
            );
        }
    }
    if args.iter().any(|a| a == "--imports") {
        // Package-qualified names outside the movie's own code: the host API it needs.
        let mut names = std::collections::BTreeSet::new();
        for m in &abc.constant_pool.multinames {
            if let Multiname::QName { namespace: ns, name } = m {
                let ns = namespace(&abc, ns);
                if ns.starts_with("flash.") || ns.starts_with("iggy") || ns == "__AS3__.vec" {
                    names.insert(format!("{ns}:{}", string(&abc, name)));
                }
            }
        }
        for name in names {
            println!("{name}");
        }
    }
    if args.iter().any(|a| a == "--ops") {
        let mut ops = BTreeMap::<String, usize>::new();
        for body in &abc.method_bodies {
            let mut reader = Reader::new(&body.code);
            while reader.pos(&body.code) < body.code.len() {
                let op = reader.read_op().map_err(|e| e.to_string())?;
                *ops.entry(op_name(&op)).or_default() += 1;
            }
        }
        let mut ops: Vec<_> = ops.into_iter().collect();
        ops.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        for (name, n) in ops {
            println!("{n:8} {name}");
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--disassemble") {
        let wanted = args.get(i + 1).ok_or("--disassemble needs a class name")?;
        for (index, instance) in abc.instances.iter().enumerate() {
            let name = multiname(&abc, &instance.name);
            if !name.ends_with(wanted.as_str()) {
                continue;
            }
            let class = &abc.classes[index];
            let mut methods = vec![("<init>".to_owned(), instance.init_method.0)];
            methods.push(("<cinit>".to_owned(), class.init_method.0));
            for trait_ in instance.traits.iter().chain(&class.traits) {
                use swf::avm2::types::TraitKind;
                if let TraitKind::Method { method, .. }
                | TraitKind::Getter { method, .. }
                | TraitKind::Setter { method, .. } = &trait_.kind
                {
                    methods.push((multiname(&abc, &trait_.name), method.0));
                }
            }
            println!("== {name}");
            for (method_name, method) in methods {
                let Some(body) = abc.method_bodies.iter().find(|b| b.method.0 == method) else {
                    continue;
                };
                println!("-- {method_name} (locals {}, stack {})", body.num_locals, body.max_stack);
                let mut reader = Reader::new(&body.code);
                while reader.pos(&body.code) < body.code.len() {
                    let at = reader.pos(&body.code);
                    let op = reader.read_op().map_err(|e| e.to_string())?;
                    let text = match &op {
                        Op::GetProperty { index }
                        | Op::SetProperty { index }
                        | Op::InitProperty { index }
                        | Op::FindPropStrict { index }
                        | Op::FindProperty { index }
                        | Op::GetLex { index }
                        | Op::GetSuper { index }
                        | Op::SetSuper { index } => format!("{} {}", op_name(&op), multiname(&abc, index)),
                        Op::CallProperty { index, num_args }
                        | Op::CallPropVoid { index, num_args }
                        | Op::ConstructProp { index, num_args }
                        | Op::CallSuper { index, num_args }
                        | Op::CallSuperVoid { index, num_args } => {
                            format!("{} {} ({num_args})", op_name(&op), multiname(&abc, index))
                        }
                        Op::PushString { value } => format!("PushString {:?}", string(&abc, value)),
                        other => format!("{other:?}"),
                    };
                    println!("{at:6} {text}");
                }
            }
        }
    }
    Ok(())
}
