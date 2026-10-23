"""Generate src/platform/cvar_table.rs from the idCVar registrations in Dishonored2.exe.

Every static idCVar is registered by one of three constructors (all reach
FUN_140101640):
  FUN_1401008f0(cvar, name, default, flags, desc, cb)            range (1, -1)
  FUN_140100840(cvar, name, default, flags, desc, min, max, cb)
  FUN_1401009b0(cvar, name, default, flags, desc, values, cb)    range (1, -1)
The calls are read from the Ghidra export (analysis/decompiled/chunk_*.c);
string, float and value-list operands are read from the executable.
Registration order is ascending cvar address (the static initializers follow
the data layout; dishonored2Config.cfg lists its cvars in exactly the reverse
order), so the table is sorted by address.
Usage: python -I analysis/generate_cvar_table.py <decompiled dir> <Dishonored2.exe> <out.rs>
"""
import glob
import json
import os
import re
import struct
import sys

import pefile

src, exe, out = sys.argv[1], sys.argv[2], sys.argv[3]
pe = pefile.PE(exe, fast_load=True)
base = pe.OPTIONAL_HEADER.ImageBase
data = open(exe, 'rb').read()


def offset(va):
    for s in pe.sections:
        start = base + s.VirtualAddress
        if start <= va < start + max(s.Misc_VirtualSize, s.SizeOfRawData):
            return s.PointerToRawData + va - start
    raise ValueError(hex(va))


def read(va, n):
    o = offset(va)
    return data[o:o + n]


def cstr(va):
    o = offset(va)
    return data[o:data.index(b'\0', o)].decode('latin1')


def arguments(text, start):
    """Split a C call's arguments starting after '('; returns (args, end)."""
    args, cur, depth, i = [], '', 0, start
    while True:
        c = text[i]
        if c == '"':
            j = i + 1
            while text[j] != '"':
                j += 2 if text[j] == '\\' else 1
            cur += text[i:j + 1]
            i = j + 1
            continue
        if c == '(':
            depth += 1
        elif c == ')':
            if depth == 0:
                args.append(cur.strip())
                return args, i
            depth -= 1
        elif c == ',' and depth == 0:
            args.append(cur.strip())
            cur = ''
            i += 1
            continue
        cur += c
        i += 1


def c_string(token):
    body = token[1:-1]
    return body.encode('latin1').decode('unicode_escape')


def string(token):
    if token.startswith('"'):
        return c_string(token)
    va = int(re.search(r'([0-9a-f]{9})$', token).group(1), 16)
    # Without '&' the operand is a pointer variable's value (PTR_s_...).
    if not token.startswith('&'):
        va = struct.unpack('<Q', read(va, 8))[0]
    return cstr(va)


def number(token):
    token = token.strip().rstrip('U')
    if token.startswith('(') and ')' in token:
        token = token[token.index(')') + 1:]
    return int(token, 0)


def float_operand(token):
    m = re.search(r'DAT_([0-9a-f]{9})$', token)
    if m:
        return struct.unpack('<f', read(int(m.group(1), 16), 4))[0]
    # A literal is the raw float bits of the undefined4 parameter.
    return struct.unpack('<f', struct.pack('<I', number(token) & 0xffffffff))[0]


def values(token):
    if token in ('0', '(char **)0x0', '(undefined8 *)0x0'):
        return []
    m = re.search(r'([0-9a-f]{9})$', token)
    va = int(m.group(1), 16)
    out = []
    while True:
        p = struct.unpack('<Q', read(va, 8))[0]
        # The game walks until an empty string (FUN_140102cb0); some lists
        # end in a non-pointer instead, which only a match before it avoids.
        try:
            text = cstr(p)
        except ValueError:
            return out
        if text == '':
            return out
        out.append(text)
        va += 8


def flags_after_init(f):
    """FUN_140101640's flag rewrite."""
    if f & 8:
        f |= 0x20
    if f & 0x3c010 == 0:
        f |= 8
    else:
        f = f & 0xfffffff7 | 0x10
    return f


def parse(fn, args):
    address = int(re.search(r'([0-9a-f]{9})$', args[0]).group(1), 16)
    name = string(args[1])
    default = string(args[2])
    flags = flags_after_init(number(args[3]))
    lo, hi, vals = 1.0, -1.0, []
    if fn == 'FUN_140100840':
        lo, hi = float_operand(args[5]), float_operand(args[6])
    elif fn == 'FUN_1401009b0':
        vals = values(args[5])
    if address in cvars and cvars[address][0] != name:
        raise SystemExit('two names for cvar %x: %s %s' % (address, cvars[address][0], name))
    cvars[address] = (name, default, flags, lo, hi, vals)


calls = re.compile(r'\b(FUN_1401008f0|FUN_140100840|FUN_1401009b0)\(&DAT_([0-9a-f]{9}),')
cvars = {}
for path in sorted(glob.glob(os.path.join(src, 'chunk_*.c'))):
    text = open(path, encoding='utf8', errors='replace').read()
    for m in calls.finditer(text):
        args, _ = arguments(text, m.end(1) + 1)
        fn = m.group(1)
        try:
            parse(fn, args)
        except Exception as e:
            raise SystemExit('%s: %s %r: %s' % (path, fn, args, e))
        continue


def rs(s):
    return json.dumps(s, ensure_ascii=True)


def rf(v):
    r = repr(float(v))
    return r if r not in ('inf', '-inf') else ('f32::INFINITY' if v > 0 else 'f32::NEG_INFINITY')


with open(out, 'w', encoding='utf8', newline='\n') as f:
    f.write('//! Generated by analysis/generate_cvar_table.py from Dishonored2.exe.\n')
    f.write('//! Do not edit by hand.\n\n')
    f.write('/// Static idCVar registrations in registration order (ascending cvar\n')
    f.write('/// address): `(name, default, flags after FUN_140101640, min, max, values)`.\n')
    f.write('pub const CVARS: &[(&str, &str, u32, f32, f32, &[&str])] = &[\n')
    for address in sorted(cvars):
        name, default, flags, lo, hi, vals = cvars[address]
        f.write('    (%s, %s, %#x, %s, %s, &[%s]),\n' % (rs(name), rs(default), flags, rf(lo), rf(hi), ', '.join(rs(v) for v in vals)))
    f.write('];\n')
print(len(cvars), 'cvars')
