// Rewrite-authored PC LightContext iterator from lightcontext.renderprog.decl.
// The producer supplies the original four horizontal head planes. The list
// limit is explicit, as in NextLightData; no sorting or dropped-kind fallback.
#define_import_path dh2_tlf_list

const END_OF_LIGHT_LIST: u32 = 0x007fffffu;

struct LightList {
    head: u32,
    count: u32,
}

fn light_head_coordinate(pixel: vec2<u32>, light_type: u32,
                         tile_shift: u32, tile_width: u32) -> vec2<u32> {
    return vec2<u32>((pixel.x >> tile_shift) + light_type * tile_width,
                     pixel.y >> tile_shift);
}

fn begin_light_list(head: u32) -> LightList {
    return LightList(head, 0u);
}

fn has_light(list: LightList, maximum: u32) -> bool {
    return list.head != END_OF_LIGHT_LIST && list.count < maximum;
}

// The caller fetches the current pool word before advancing. Separating the
// resource load keeps the same iterator usable with the actual view pool and
// with independently executed GPU fixtures, without dummy global bindings.
fn advance_light(list: ptr<function, LightList>, packed_node: u32) -> u32 {
    (*list).head = packed_node >> 9u;
    (*list).count += 1u;
    return packed_node & 511u;
}
