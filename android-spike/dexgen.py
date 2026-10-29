#!/usr/bin/env python3
"""A minimal DEX (format 035) writer, the prototype of the plan's dex.rs.

Writes the classes the framework must find as DEX, with every
framework-called method `native` (no bytecode) and only constructors and
SDLActivity.<clinit> carrying code:

  gen/classes.dex:
    org.libsdl.app.SDLActivity extends android.app.Activity
        static <clinit>  { System.loadLibrary("main"); }
        public <init>()  { super(); }
        native onCreate(Bundle), onResume(), onPause(), onDestroy(),
               onWindowFocusChanged(boolean), dispatchTouchEvent(MotionEvent),
               dispatchKeyEvent(KeyEvent), dispatchGenericMotionEvent(MotionEvent)
    org.libsdl.app.SDLSurface extends android.view.SurfaceView
                              implements android.view.SurfaceHolder.Callback
        public <init>(Context) { super(context); }
        native surfaceCreated/surfaceChanged/surfaceDestroyed
    org.libsdl.app.SDLNativeRunnable implements java.lang.Runnable
        public long ptr;  public <init>() { super(); }  native run()
  gen/inmem.dex (loaded with InMemoryDexClassLoader):
    org.libsdl.inmem.SDLInMemoryThread extends java.lang.Thread
        public <init>() { super(); }  native run()

Usage: dexgen.py OUTDIR    (also: dexgen.py --dump FILE.dex)
"""

import hashlib
import struct
import sys
import zlib

ACC_PUBLIC, ACC_PRIVATE, ACC_PROTECTED, ACC_STATIC, ACC_FINAL = 0x1, 0x2, 0x4, 0x8, 0x10
ACC_NATIVE, ACC_CONSTRUCTOR = 0x100, 0x10000
NO_INDEX = 0xFFFFFFFF


def uleb(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def mutf8(s):
    out = bytearray()
    for ch in s:
        c = ord(ch)
        if c == 0:
            out += b"\xc0\x80"
        elif c < 0x80:
            out.append(c)
        elif c < 0x800:
            out += bytes([0xC0 | (c >> 6), 0x80 | (c & 0x3F)])
        elif c < 0x10000:
            out += bytes([0xE0 | (c >> 12), 0x80 | ((c >> 6) & 0x3F), 0x80 | (c & 0x3F)])
        else:  # surrogate pair, each as 3 bytes
            c -= 0x10000
            for u in (0xD800 | (c >> 10), 0xDC00 | (c & 0x3FF)):
                out += bytes([0xE0 | (u >> 12), 0x80 | ((u >> 6) & 0x3F), 0x80 | (u & 0x3F)])
    return bytes(out)


def utf16_key(s):
    return list(s.encode("utf-16-be"))


def shorty_char(t):
    return "L" if t[0] in "L[" else t


class Method:
    def __init__(self, cls, name, ret, params, access, code=None):
        self.cls, self.name, self.ret, self.params = cls, name, ret, tuple(params)
        self.access, self.code = access, code  # code: (registers, ins, outs, insn-builder)

    @property
    def proto(self):
        return (self.ret, self.params)


class Dex:
    def __init__(self):
        self.classes = []  # (name, access, super, interfaces, fields, methods)
        self.method_refs = set()  # (cls, name, ret, params)
        self.field_refs = set()  # (cls, type, name)
        self.strings = set()

    def add_class(self, name, access, superclass, interfaces=(), fields=(), methods=()):
        self.classes.append((name, access, superclass, tuple(interfaces), list(fields), list(methods)))
        for m in methods:
            self.method_refs.add((m.cls, m.name, m.ret, m.params))
        for f in fields:  # (name, type, access)
            self.field_refs.add((name, f[1], f[0]))

    def ref_method(self, cls, name, ret, params):
        self.method_refs.add((cls, name, ret, tuple(params)))
        return (cls, name, ret, tuple(params))

    def build(self):
        # ---- collect ids
        types = set()
        protos = set()
        for (cls, name, ret, params) in self.method_refs:
            types.add(cls); types.add(ret); types.update(params)
            self.strings.add(name)
            protos.add((ret, params))
        for (cls, t, name) in self.field_refs:
            types.add(cls); types.add(t); self.strings.add(name)
        for c in self.classes:
            types.add(c[0]); types.add(c[2]); types.update(c[3])
        for (ret, params) in protos:
            self.strings.add(shorty_char(ret) + "".join(shorty_char(p) for p in params))
        self.strings.update(types)

        strings = sorted(self.strings, key=utf16_key)
        sidx = {s: i for i, s in enumerate(strings)}
        type_list = sorted(types, key=lambda t: sidx[t])
        tidx = {t: i for i, t in enumerate(type_list)}
        proto_list = sorted(protos, key=lambda p: (tidx[p[0]], [tidx[x] for x in p[1]]))
        pidx = {p: i for i, p in enumerate(proto_list)}
        field_list = sorted(self.field_refs, key=lambda f: (tidx[f[0]], sidx[f[2]], tidx[f[1]]))
        fidx = {f: i for i, f in enumerate(field_list)}
        meth_list = sorted(self.method_refs, key=lambda m: (tidx[m[0]], sidx[m[1]], pidx[(m[2], m[3])]))
        midx = {m: i for i, m in enumerate(meth_list)}
        self.sidx, self.midx = sidx, midx

        # ---- layout
        header_size = 0x70
        off = header_size
        string_ids_off = off; off += 4 * len(strings)
        type_ids_off = off; off += 4 * len(type_list)
        proto_ids_off = off; off += 12 * len(proto_list)
        field_ids_off = off; off += 8 * len(field_list)
        method_ids_off = off; off += 8 * len(meth_list)
        class_defs_off = off; off += 32 * len(self.classes)
        data_off = off

        data = bytearray()

        def align4():
            while (data_off + len(data)) % 4:
                data.append(0)

        def cur():
            return data_off + len(data)

        # code items
        code_off = {}
        n_code = 0
        code_start = None
        for c in self.classes:
            for m in c[5]:
                if m.code is not None:
                    align4()
                    if code_start is None:
                        code_start = cur()
                    regs, ins, outs, builder = m.code
                    insns = builder(self)
                    code_off[(m.cls, m.name, m.ret, m.params)] = cur()
                    data += struct.pack("<HHHHII", regs, ins, outs, 0, 0, len(insns))
                    data += struct.pack("<%dH" % len(insns), *insns)
                    n_code += 1
        # type lists (proto params, interfaces), deduplicated
        align4()
        tl_off = {}
        tl_start = None
        lists = []
        for p in proto_list:
            if p[1]:
                lists.append(p[1])
        for c in self.classes:
            if c[3]:
                lists.append(c[3])
        for l in lists:
            if l in tl_off:
                continue
            align4()
            if tl_start is None:
                tl_start = cur()
            tl_off[l] = cur()
            data += struct.pack("<I", len(l)) + struct.pack("<%dH" % len(l), *[tidx[t] for t in l])
        n_tl = len(tl_off)
        # string data
        sd_start = cur()
        sd_off = []
        for s in strings:
            sd_off.append(cur())
            data += uleb(len(s.encode("utf-16-le")) // 2) + mutf8(s) + b"\0"
        # class data
        cd_start = cur()
        cd_off = []
        for (name, access, sup, ifaces, fields, methods) in self.classes:
            cd_off.append(cur())
            sfields = sorted([f for f in fields if f[2] & ACC_STATIC], key=lambda f: fidx[(name, f[1], f[0])])
            ifields = sorted([f for f in fields if not f[2] & ACC_STATIC], key=lambda f: fidx[(name, f[1], f[0])])
            key = lambda m: midx[(m.cls, m.name, m.ret, m.params)]
            direct = sorted([m for m in methods if m.access & (ACC_STATIC | ACC_PRIVATE | ACC_CONSTRUCTOR)], key=key)
            virtual = sorted([m for m in methods if not m.access & (ACC_STATIC | ACC_PRIVATE | ACC_CONSTRUCTOR)], key=key)
            data += uleb(len(sfields)) + uleb(len(ifields)) + uleb(len(direct)) + uleb(len(virtual))
            for group in (sfields, ifields):
                prev = 0
                for f in group:
                    i = fidx[(name, f[1], f[0])]
                    data += uleb(i - prev) + uleb(f[2]); prev = i
            for group in (direct, virtual):
                prev = 0
                for m in group:
                    k = (m.cls, m.name, m.ret, m.params)
                    i = midx[k]
                    data += uleb(i - prev) + uleb(m.access) + uleb(code_off.get(k, 0)); prev = i
        # map list
        align4()
        map_off = cur()
        items = [(0x0000, 1, 0), (0x0001, len(strings), string_ids_off), (0x0002, len(type_list), type_ids_off),
                 (0x0003, len(proto_list), proto_ids_off)]
        if field_list:
            items.append((0x0004, len(field_list), field_ids_off))
        items += [(0x0005, len(meth_list), method_ids_off), (0x0006, len(self.classes), class_defs_off)]
        if n_code:
            items.append((0x2001, n_code, code_start))
        if n_tl:
            items.append((0x1001, n_tl, tl_start))
        items += [(0x2002, len(strings), sd_start), (0x2000, len(self.classes), cd_start), (0x1000, 1, map_off)]
        items.sort(key=lambda x: x[2])
        data += struct.pack("<I", len(items))
        for (t, n, o) in items:
            data += struct.pack("<HHII", t, 0, n, o)
        file_size = data_off + len(data)

        # ---- ids
        ids = bytearray()
        for o in sd_off:
            ids += struct.pack("<I", o)
        for t in type_list:
            ids += struct.pack("<I", sidx[t])
        for (ret, params) in proto_list:
            shorty = shorty_char(ret) + "".join(shorty_char(p) for p in params)
            ids += struct.pack("<III", sidx[shorty], tidx[ret], tl_off[params] if params else 0)
        for (cls, t, name) in field_list:
            ids += struct.pack("<HHI", tidx[cls], tidx[t], sidx[name])
        for (cls, name, ret, params) in meth_list:
            ids += struct.pack("<HHI", tidx[cls], pidx[(ret, params)], sidx[name])
        for i, (name, access, sup, ifaces, fields, methods) in enumerate(self.classes):
            ids += struct.pack("<IIIIIIII", tidx[name], access, tidx[sup], tl_off[ifaces] if ifaces else 0,
                               NO_INDEX, 0, cd_off[i], 0)
        assert header_size + len(ids) == data_off

        hdr = bytearray(b"dex\n035\0" + b"\0" * 24)
        hdr += struct.pack("<IIIIII", file_size, header_size, 0x12345678, 0, 0, map_off)
        for (n, o) in ((len(strings), string_ids_off), (len(type_list), type_ids_off),
                       (len(proto_list), proto_ids_off), (len(field_list), field_ids_off if field_list else 0),
                       (len(meth_list), method_ids_off), (len(self.classes), class_defs_off),
                       (len(data), data_off)):
            hdr += struct.pack("<II", n, o)
        assert len(hdr) == header_size
        out = bytearray(hdr + ids + data)
        out[12:32] = hashlib.sha1(out[32:]).digest()
        out[8:12] = struct.pack("<I", zlib.adler32(bytes(out[12:])) & 0xFFFFFFFF)
        return bytes(out)


# ---- code builders (registers, ins, outs, builder)
def ctor_code(dex, cls, sup, params=()):
    m = dex.ref_method(sup, "<init>", "V", params)
    n = 1 + len(params)
    assert n <= 4 and all(p not in ("J", "D") for p in params)

    def b(d):
        regs = 0
        for i in range(n):
            regs |= i << (4 * i)
        return [(n << 12) | 0x70, d.midx[m], regs, 0x000E]  # invoke-direct {v0..}, super.<init>; return-void
    return (n, n, n, b)


def clinit_code(dex, lib):
    m = dex.ref_method("Ljava/lang/System;", "loadLibrary", "V", ["Ljava/lang/String;"])
    dex.strings.add(lib)

    def b(d):
        # const-string v0, lib; invoke-static {v0}, System.loadLibrary; return-void
        return [0x001A, d.sidx[lib], 0x1071, d.midx[m], 0x0000, 0x000E]
    return (1, 0, 1, b)


def classes_dex():
    d = Dex()
    A = "Lorg/libsdl/app/SDLActivity;"
    nat = ACC_NATIVE
    d.add_class(A, ACC_PUBLIC, "Landroid/app/Activity;", methods=[
        Method(A, "<clinit>", "V", [], ACC_STATIC | ACC_CONSTRUCTOR, clinit_code(d, "main")),
        Method(A, "<init>", "V", [], ACC_PUBLIC | ACC_CONSTRUCTOR, ctor_code(d, A, "Landroid/app/Activity;")),
        Method(A, "onCreate", "V", ["Landroid/os/Bundle;"], ACC_PROTECTED | nat),
        Method(A, "onResume", "V", [], ACC_PROTECTED | nat),
        Method(A, "onPause", "V", [], ACC_PROTECTED | nat),
        Method(A, "onDestroy", "V", [], ACC_PROTECTED | nat),
        Method(A, "onWindowFocusChanged", "V", ["Z"], ACC_PUBLIC | nat),
        Method(A, "dispatchTouchEvent", "Z", ["Landroid/view/MotionEvent;"], ACC_PUBLIC | nat),
        Method(A, "dispatchKeyEvent", "Z", ["Landroid/view/KeyEvent;"], ACC_PUBLIC | nat),
        Method(A, "dispatchGenericMotionEvent", "Z", ["Landroid/view/MotionEvent;"], ACC_PUBLIC | nat),
    ])
    S = "Lorg/libsdl/app/SDLSurface;"
    H = "Landroid/view/SurfaceHolder;"
    d.add_class(S, ACC_PUBLIC, "Landroid/view/SurfaceView;", ["Landroid/view/SurfaceHolder$Callback;"], methods=[
        Method(S, "<init>", "V", ["Landroid/content/Context;"], ACC_PUBLIC | ACC_CONSTRUCTOR,
               ctor_code(d, S, "Landroid/view/SurfaceView;", ["Landroid/content/Context;"])),
        Method(S, "surfaceCreated", "V", [H], ACC_PUBLIC | nat),
        Method(S, "surfaceChanged", "V", [H, "I", "I", "I"], ACC_PUBLIC | nat),
        Method(S, "surfaceDestroyed", "V", [H], ACC_PUBLIC | nat),
    ])
    R = "Lorg/libsdl/app/SDLNativeRunnable;"
    d.add_class(R, ACC_PUBLIC, "Ljava/lang/Object;", ["Ljava/lang/Runnable;"], fields=[("ptr", "J", ACC_PUBLIC)],
                methods=[
        Method(R, "<init>", "V", [], ACC_PUBLIC | ACC_CONSTRUCTOR, ctor_code(d, R, "Ljava/lang/Object;")),
        Method(R, "run", "V", [], ACC_PUBLIC | nat),
    ])
    return d.build()


def inmem_dex():
    d = Dex()
    T = "Lorg/libsdl/inmem/SDLInMemoryThread;"
    d.add_class(T, ACC_PUBLIC, "Ljava/lang/Thread;", methods=[
        Method(T, "<init>", "V", [], ACC_PUBLIC | ACC_CONSTRUCTOR, ctor_code(d, T, "Ljava/lang/Thread;")),
        Method(T, "run", "V", [], ACC_PUBLIC | ACC_NATIVE),
    ])
    return d.build()


# ---- a small reader, to check the output here (no dexdump on this host)
def dump(b):
    def u4(o): return struct.unpack_from("<I", b, o)[0]
    def u2(o): return struct.unpack_from("<H", b, o)[0]

    def ruleb(o):
        r = s = 0
        while True:
            x = b[o]; o += 1
            r |= (x & 0x7F) << s; s += 7
            if not x & 0x80:
                return r, o
    assert b[:8] == b"dex\n035\0", b[:8]
    assert u4(8) == zlib.adler32(b[12:]) & 0xFFFFFFFF, "adler32"
    assert b[12:32] == hashlib.sha1(b[32:]).digest(), "sha1"
    assert u4(32) == len(b), "file_size"
    ns, so, nt, to, np_, po, nf, fo, nm, mo, nc, co, ds, do = struct.unpack_from("<14I", b, 56)

    def string(i):
        o = u4(so + 4 * i)
        _, o = ruleb(o)
        e = b.index(b"\0", o)
        return b[o:e].decode("utf-8", "replace")
    strs = [string(i) for i in range(ns)]
    assert strs == sorted(strs, key=utf16_key), "strings unsorted"
    tys = [strs[u4(to + 4 * i)] for i in range(nt)]

    def tlist(o):
        return [tys[u2(o + 4 + 2 * k)] for k in range(u4(o))] if o else []
    protos = []
    for i in range(np_):
        sh, rt, pa = struct.unpack_from("<III", b, po + 12 * i)
        protos.append((strs[sh], tys[rt], tlist(pa)))
    meths = []
    for i in range(nm):
        c, p, n = struct.unpack_from("<HHI", b, mo + 8 * i)
        meths.append("%s->%s(%s)%s" % (tys[c], strs[n], "".join(protos[p][2]), protos[p][1]))
    fields = []
    for i in range(nf):
        c, t, n = struct.unpack_from("<HHI", b, fo + 8 * i)
        fields.append("%s->%s:%s" % (tys[c], strs[n], tys[t]))
    print("dex %d bytes: %d strings, %d types, %d protos, %d fields, %d methods, %d classes, data %d@0x%x"
          % (len(b), ns, nt, np_, nf, nm, nc, ds, do))
    for i, m in enumerate(meths):
        print("  method[%d] %s" % (i, m))
    for i in range(nc):
        ci, acc, sup, ifo, _, _, cdo, _ = struct.unpack_from("<8I", b, co + 32 * i)
        print("class %s access=0x%x super=%s interfaces=%s" % (tys[ci], acc, tys[sup], tlist(ifo)))
        o = cdo
        sf, o = ruleb(o); inf, o = ruleb(o); dm, o = ruleb(o); vm, o = ruleb(o)
        for kind, n in (("static field", sf), ("field", inf)):
            idx = 0
            for _ in range(n):
                d, o = ruleb(o); a, o = ruleb(o); idx += d
                print("  %s %s access=0x%x" % (kind, fields[idx], a))
        for kind, n in (("direct", dm), ("virtual", vm)):
            idx = 0
            for _ in range(n):
                d, o = ruleb(o); a, o = ruleb(o); c, o = ruleb(o); idx += d
                line = "  %s %s access=0x%x" % (kind, meths[idx], a)
                if c:
                    regs, ins, outs, tries, dbg, n_ins = struct.unpack_from("<HHHHII", b, c)
                    insns = struct.unpack_from("<%dH" % n_ins, b, c + 16)
                    line += " code@0x%x regs=%d ins=%d outs=%d insns=%s" % (
                        c, regs, ins, outs, " ".join("%04x" % x for x in insns))
                print(line)
    mo_ = u4(52)
    for k in range(u4(mo_)):
        t, _, n, o = struct.unpack_from("<HHII", b, mo_ + 4 + 12 * k)
        print("  map type=0x%04x size=%d off=0x%x" % (t, n, o))


if __name__ == "__main__":
    if sys.argv[1] == "--dump":
        dump(open(sys.argv[2], "rb").read())
        sys.exit(0)
    import os
    os.makedirs(sys.argv[1], exist_ok=True)
    for name, data in (("classes.dex", classes_dex()), ("inmem.dex", inmem_dex())):
        p = os.path.join(sys.argv[1], name)
        with open(p, "wb") as f:
            f.write(data)
        print("wrote %s (%d bytes, sha256 %s)" % (p, len(data), hashlib.sha256(data).hexdigest()[:16]))
