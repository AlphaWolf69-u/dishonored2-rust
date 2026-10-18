"""Capture native optimized shadow-PCF results from the original HLSL on D3D11.

The probe extracts the installed original function bodies at runtime, compiles
those bodies with FXC, seeds a synthetic R24G8_TYPELESS depth-array texture,
and writes only synthetic depth codes, inputs, and GPU results to the fixture.
No game shader source or copyrighted shader bytecode is written to disk.
"""
from __future__ import annotations

import ctypes as C
import re
import struct
from pathlib import Path

from probe_original_indirect_lighting import (
    BufferDesc,
    Mapped,
    UAVDesc,
    check,
    function,
    method,
    I,
    P,
    U,
)

ROOT = Path(__file__).resolve().parents[1]
WIDTH = HEIGHT = 8
LAYERS = 2
DEPTH_MAX = (1 << 24) - 1


class Texture2DDesc(C.Structure):
    _fields_ = [
        ("width", U), ("height", U), ("mip_levels", U), ("array_size", U),
        ("format", U), ("sample_count", U), ("sample_quality", U),
        ("usage", U), ("bind_flags", U), ("cpu_access_flags", U), ("misc_flags", U),
    ]


class SubresourceData(C.Structure):
    _fields_ = [("data", P), ("row_pitch", U), ("slice_pitch", U)]


class TextureArraySrvDesc(C.Structure):
    _fields_ = [
        ("format", U), ("dimension", U), ("most_detailed_mip", U),
        ("mip_levels", U), ("first_array_slice", U), ("array_size", U),
    ]


class SamplerDesc(C.Structure):
    _fields_ = [
        ("filter", U), ("address_u", U), ("address_v", U), ("address_w", U),
        ("mip_lod_bias", C.c_float), ("max_anisotropy", U), ("comparison", U),
        ("border_color", C.c_float * 4), ("min_lod", C.c_float), ("max_lod", C.c_float),
    ]


class BufferSrvDesc(C.Structure):
    _fields_ = [
        ("format", U), ("dimension", U), ("first_element", U),
        ("num_elements", U), ("reserved0", U), ("reserved1", U),
    ]


def f32(value: float) -> float:
    return struct.unpack("<f", struct.pack("<f", value))[0]


def float_bits(value: float) -> int:
    return struct.unpack("<I", struct.pack("<f", value))[0]


def synthetic_depth_codes() -> list[list[int]]:
    """Two deterministic quantized checker/ramp layers, stored as D24 codes."""
    layers = []
    for layer in range(LAYERS):
        codes = []
        for y in range(HEIGHT):
            for x in range(WIDTH):
                if (x + y + layer) & 1:
                    depth = 0.17 + 0.055 * x + 0.028 * y + 0.13 * layer
                else:
                    depth = 0.72 - 0.038 * x + 0.017 * y - 0.08 * layer
                codes.append(int(depth * DEPTH_MAX + 0.5))
        layers.append(codes)
    return layers


def synthetic_cases():
    # Texel-space positions include half/integer boundaries and both clamp
    # edges. Dividing by eight creates the exact normalized f32 inputs below.
    texel_positions = [
        (-1.0, 2.0), (-0.5, -0.5), (0.0, 0.0), (0.25, 0.75),
        (0.5, 0.5), (0.875, 1.125), (1.0, 1.0), (1.5, 2.5),
        (2.25, 3.5), (3.0, 4.0), (3.5, 4.5), (4.5, 2.5),
        (5.25, 6.75), (6.5, 6.5), (7.5, 7.5), (8.0, 8.0),
        (0.25, 7.75), (7.75, 0.25), (-0.5, 8.5), (8.5, -0.5),
    ]
    thresholds = [0.15, 0.24, 0.32, 0.40, 0.48, 0.56, 0.64, 0.72, 0.80, 0.88]
    cases = []
    for layer in range(LAYERS):
        for index, (x, y) in enumerate(texel_positions):
            cases.append((
                f32(x / WIDTH), f32(y / HEIGHT),
                f32(thresholds[(index * 3 + layer * 5) % len(thresholds)]),
                layer,
            ))
    # Perspective division can produce non-finite coordinates at a light's
    # projection plane. Capture the actual HLSL saturate/PCF result as well.
    cases.extend([
        (float('nan'),.5,.5,0),(.5,float('nan'),.5,1),
        (float('inf'),.5,.5,0),(float('-inf'),.5,.5,1),
        (.5,float('inf'),.5,0),(.5,float('-inf'),.5,1),
        (.5,.5,float('nan'),0),(.5,.5,float('inf'),0),(.5,.5,float('-inf'),1),
    ])
    return cases


def original_hlsl(shadow_decl: str) -> bytes:
    # Extract the original tap and optimized-PCF bodies at runtime. Only the
    # original sampler resource placeholder is rebound to this synthetic map.
    sample_tap = function(shadow_decl, r"float\s+SampleShadowMap\s*\(")
    optimized_pcf = function(shadow_decl, r"float\s+SampleShadowMapOptimizedPCF\s*\(")
    sample_tap = sample_tap.replace("$(shadowHardwarePCFSampler)", "shadowComparison")
    optimized_pcf = optimized_pcf.replace("$(shadowHardwarePCFSampler)", "shadowComparison")
    for name, body in (("SampleShadowMap", sample_tap), ("SampleShadowMapOptimizedPCF", optimized_pcf)):
        if "$" in body:
            raise RuntimeError(f"unbound original resource placeholder remains in {name}")
    return "\n".join([
        "Texture2DArray<float> shadowMap : register(t0);",
        "SamplerComparisonState shadowComparison : register(s0);",
        "StructuredBuffer<float4> cases : register(t1);",
        "RWStructuredBuffer<float> results : register(u0);",
        sample_tap,
        optimized_pcf,
        """
        [numthreads(1,1,1)] void main(uint3 tid : SV_DispatchThreadID) {
            float4 p = cases[tid.x];
            results[tid.x * 3 + 0] = SampleShadowMapOptimizedPCF(
                shadowMap, p.xyz, (uint)p.w, float2(8.0, 8.0), 0);
            results[tid.x * 3 + 1] = SampleShadowMapOptimizedPCF(
                shadowMap, p.xyz, (uint)p.w, float2(8.0, 8.0), 1);
            results[tid.x * 3 + 2] = SampleShadowMapOptimizedPCF(
                shadowMap, p.xyz, (uint)p.w, float2(8.0, 8.0), 2);
        }
        """,
    ]).encode("utf8")


def compile_shader(compiler, source: bytes):
    compile_function = compiler.D3DCompile
    compile_function.argtypes = [P, C.c_size_t, C.c_char_p, P, P, C.c_char_p, C.c_char_p,
                                 U, U, C.POINTER(P), C.POINTER(P)]
    compile_function.restype = I
    code, errors = P(), P()
    hr = compile_function(source, len(source), b"original-shadow-pcf", None, None,
                          b"main", b"cs_5_0", 0x800, 0, C.byref(code), C.byref(errors))
    if errors:
        data = C.string_at(method(errors, 3, P)(errors), method(errors, 4, C.c_size_t)(errors))
        message = data.decode("utf8", errors="replace")
        method(errors, 2, U)(errors)
        if hr < 0:
            raise RuntimeError(message)
        if message.strip():
            print(message.strip())
    check(hr)
    return code


def create_buffer(device, desc: BufferDesc, initial=None):
    result = P()
    check(method(device, 3, I, C.POINTER(BufferDesc), P, C.POINTER(P))(
        device, C.byref(desc), initial, C.byref(result)))
    return result


def capture(fixture: Path | None = None):
    shadow_decl = (ROOT / "artifacts/shadow.renderprog.original.decl").read_text(encoding="utf8", errors="replace")
    codes = synthetic_depth_codes()
    cases = synthetic_cases()

    # D24 depth occupies the low 24 bits of each R24G8_TYPELESS texel; the
    # high stencil byte is zero. D3D11 initial data has one subresource per slice.
    layer_data = [C.create_string_buffer(b"".join(struct.pack("<I", code) for code in layer))
                  for layer in codes]
    initial = (SubresourceData * LAYERS)(*(
        SubresourceData(C.addressof(data), WIDTH * 4, WIDTH * HEIGHT * 4)
        for data in layer_data
    ))
    texture_desc = Texture2DDesc(WIDTH, HEIGHT, 1, LAYERS, 44, 1, 0, 0, 0x8, 0, 0)
    case_data = C.create_string_buffer(b"".join(
        struct.pack("<4f", *case[:3], float(case[3])) for case in cases
    ))

    d3d = C.WinDLL("d3d11")
    create_device = d3d.D3D11CreateDevice
    create_device.argtypes = [P, U, P, U, P, U, U, C.POINTER(P), C.POINTER(U), C.POINTER(P)]
    create_device.restype = I
    device, context = P(), P()
    feature_level = U()
    check(create_device(None, 1, None, 0, None, 0, 7,
                        C.byref(device), C.byref(feature_level), C.byref(context)))
    objects = [device, context]
    try:
        texture = P()
        check(method(device, 5, I, C.POINTER(Texture2DDesc), C.POINTER(SubresourceData), C.POINTER(P))(
            device, C.byref(texture_desc), initial, C.byref(texture)))
        objects.append(texture)
        texture_srv = P()
        texture_srv_desc = TextureArraySrvDesc(46, 5, 0, 1, 0, LAYERS)
        check(method(device, 7, I, P, C.POINTER(TextureArraySrvDesc), C.POINTER(P))(
            device, texture, C.byref(texture_srv_desc), C.byref(texture_srv)))
        objects.append(texture_srv)
        sampler = P()
        sampler_desc = SamplerDesc(
            0x94, 3, 3, 3, 0.0, 1, 2, (C.c_float * 4)(0.0, 0.0, 0.0, 0.0),
            0.0, 3.402823466e38,
        )
        check(method(device, 23, I, C.POINTER(SamplerDesc), C.POINTER(P))(
            device, C.byref(sampler_desc), C.byref(sampler)))
        objects.append(sampler)

        input_desc = BufferDesc(len(cases) * 16, 0, 0x8, 0, 0x40, 16)
        input_initial = SubresourceData(C.addressof(case_data), 0, 0)
        input_buffer = create_buffer(device, input_desc, C.byref(input_initial))
        objects.append(input_buffer)
        input_srv = P()
        input_srv_desc = BufferSrvDesc(0, 1, 0, len(cases), 0, 0)
        check(method(device, 7, I, P, C.POINTER(BufferSrvDesc), C.POINTER(P))(
            device, input_buffer, C.byref(input_srv_desc), C.byref(input_srv)))
        objects.append(input_srv)
        resources = (P * 2)(texture_srv, input_srv)
        sampler_views = (P * 1)(sampler)

        compiler = C.WinDLL("d3dcompiler_47")
        result_count = len(cases) * 3
        result_bytes = result_count * 4
        code = compile_shader(compiler, original_hlsl(shadow_decl))
        objects.append(code)
        shader = P()
        check(method(device, 18, I, P, C.c_size_t, P, C.POINTER(P))(
            device, method(code, 3, P)(code), method(code, 4, C.c_size_t)(code),
            None, C.byref(shader)))
        objects.append(shader)
        gpu_desc = BufferDesc(result_bytes, 0, 0x80, 0, 0x40, 4)
        gpu = create_buffer(device, gpu_desc)
        objects.append(gpu)
        output = P()
        output_desc = UAVDesc(0, 1, 0, result_count, 0)
        check(method(device, 8, I, P, C.POINTER(UAVDesc), C.POINTER(P))(
            device, gpu, C.byref(output_desc), C.byref(output)))
        objects.append(output)
        staging_desc = BufferDesc(result_bytes, 3, 0, 0x20000, 0, 0)
        staging = create_buffer(device, staging_desc)
        objects.append(staging)

        method(context, 67, None, U, U, C.POINTER(P))(context, 0, 2, resources)
        method(context, 70, None, U, U, C.POINTER(P))(context, 0, 1, sampler_views)
        method(context, 69, None, P, P, U)(context, shader, None, 0)
        output_views = (P * 1)(output)
        method(context, 68, None, U, U, C.POINTER(P), P)(context, 0, 1, output_views, None)
        method(context, 41, None, U, U, U)(context, len(cases), 1, 1)
        method(context, 47, None, P, P)(context, staging, gpu)
        mapped = Mapped()
        check(method(context, 14, I, P, U, U, U, C.POINTER(Mapped))(
            context, staging, 0, 1, 0, C.byref(mapped)))
        values = struct.unpack(f"<{result_count}f", C.string_at(mapped.data, result_bytes))
        method(context, 15, None, P, U)(context, staging, 0)

        output_lines = [
            "# Synthetic original-HLSL optimized-PCF capture; fields are f32 bit patterns.",
            f"MAP {WIDTH} {HEIGHT} {LAYERS} {DEPTH_MAX}",
        ]
        for layer, layer_codes in enumerate(codes):
            output_lines.append(f"DEPTH {layer} " + " ".join(f"{value:06x}" for value in layer_codes))
        for case_index, (x, y, depth, layer) in enumerate(cases):
            for filtering_quality in range(3):
                output_lines.append(
                    f"CASE {filtering_quality} {layer} "
                    f"{float_bits(x):08x} {float_bits(y):08x} {float_bits(depth):08x} "
                    f"{float_bits(values[case_index * 3 + filtering_quality]):08x}"
                )
        fixture = fixture or ROOT / "tests/fixtures/shadow_pcf_cases.txt"
        fixture.write_text("\n".join(output_lines) + "\n", encoding="ascii")
        print(f"Captured {len(cases) * 3} PCF cases on D3D11 feature level 0x{feature_level.value:04x}: {fixture}")
    finally:
        for obj in reversed(objects):
            method(obj, 2, U)(obj)


if __name__ == "__main__":
    capture()
