//! Demand-only DSM layout decoding over the retained Navmesh text bytes.
//!083040's reflected m_dsmLayouts list is preserved, not reconstructed from
//! navigation triangles. Unused connectivity/face tables remain borrowed.
use super::*;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Cell {
    pub(crate) key: [u16; 3],
    pub(crate) nav_face_id: u32,
    pub(crate) offset: f32,
    area_volume: Option<(u32, u32)>,
}
#[derive(Debug)]
pub(crate) struct Graph {
    source: Arc<Metadata>,
    range: Range<usize>,
    pub(crate) cells: Box<[Cell]>,
    pub(crate) grid_origin: [f32; 3],
    pub(crate) nav_mesh_id: u32,
    neighbor_offsets: Box<[u32]>,
    neighbors: Box<[u16]>,
}
impl Graph {
    pub(crate) fn neighbors(&self, cell: usize) -> Result<impl Iterator<Item = u16> + '_, String> {
        let end = *self.neighbor_offsets.get(cell + 1).ok_or("DSM neighbor cell is out of range")? as usize;
        let start = self.neighbor_offsets[cell] as usize;
        Ok(self.neighbors[start..end].iter().copied())
    }
    pub(crate) fn source_text(&self) -> Result<&str, String> {
        std::str::from_utf8(self.source.bytes(&self.range)).map_err(|error| error.to_string())
    }
    pub(crate) fn area_volume(&self, cell: &Cell) -> Result<Option<&str>, String> {
        let Some((start, end)) = cell.area_volume else { return Ok(None) };
        std::str::from_utf8(&self.source.bytes[start as usize..end as usize]).map(Some).map_err(|error| error.to_string())
    }
}

/// A borrowed reflected value, with absolute offsets into the one byte owner.
#[derive(Clone, Copy)]
struct Value<'a> { text: &'a str, start: usize }
struct Tokens<'a> { text: &'a str, at: usize }
impl<'a> Tokens<'a> {
    fn next(&mut self) -> Result<Option<(usize, &'a str, bool)>, String> {
        let bytes = self.text.as_bytes();
        while bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) { self.at += 1; }
        let start = self.at;
        let Some(&first) = bytes.get(self.at) else { return Ok(None) };
        self.at += 1;
        if first == b'"' {
            let value = self.at;
            while bytes.get(self.at).is_some_and(|byte| *byte != b'"') {
                if bytes[self.at] == b'\\' { self.at += 1; }
                self.at += 1;
            }
            if self.at >= bytes.len() { return Err("unterminated DSM string".into()); }
            let end = self.at;
            self.at += 1;
            return Ok(Some((value, &self.text[value..end], true)));
        }
        if !b"{}=;".contains(&first) {
            while bytes.get(self.at).is_some_and(|byte| !byte.is_ascii_whitespace() && !b"{}=;".contains(byte)) { self.at += 1; }
        }
        Ok(Some((start, &self.text[start..self.at], false)))
    }
}
impl<'a> Value<'a> {
    fn fields(&self, mut visit: impl FnMut(&'a str, Value<'a>) -> Result<(), String>) -> Result<(), String> {
        let mut tokens = Tokens { text: self.text, at: 0 };
        let mut name = "";
        while let Some((_, token, quoted)) = tokens.next()? {
            if !quoted && token == "=" {
                let Some((start, token, quoted)) = tokens.next()? else { return Err("DSM field value is absent".into()) };
                let (start, end) = if !quoted && token == "{" {
                    let start = tokens.at;
                    let mut depth = 1;
                    let end = loop {
                        let Some((position, token, quoted)) = tokens.next()? else { return Err("unterminated DSM object".into()) };
                        if !quoted && token == "{" { depth += 1; }
                        if !quoted && token == "}" { depth -= 1; if depth == 0 { break position; } }
                    };
                    (start, end)
                } else { (start, start + token.len()) };
                visit(name, Value { text: &self.text[start..end], start: self.start + start })?;
                name = "";
            } else if !quoted && token != ";" { name = token; }
        }
        Ok(())
    }
    fn field(&self, wanted: &str) -> Result<Option<Self>, String> {
        let mut found = None;
        self.fields(|name, value| { if name == wanted { found = Some(value); } Ok(()) })?;
        Ok(found)
    }
    fn required(&self, wanted: &str) -> Result<Self, String> {
        self.field(wanted)?.ok_or_else(|| format!("DSM field {wanted} is absent"))
    }
    fn number<T: std::str::FromStr>(&self) -> Result<T, String> {
        self.text.trim().parse().map_err(|_| format!("invalid DSM number {}", self.text))
    }
}

impl Metadata {
    pub(crate) fn dsm_graph_for(self: &Arc<Self>, world: u16, instance: u16) -> Result<Arc<Graph>, String> {
        let layer = self.layers.iter().position(|layer| layer.world == world && layer.instance == instance)
            .ok_or("requested navmesh world instance is not admitted by this resource")?;
        self.dsm_graph(layer, 0)
    }
    /// Source1b1510 binds the first actual layout at section+30 to runtime+60.
    /// The second serialized layout remains available to its actual consumer.
    pub(crate) fn dsm_graph(self: &Arc<Self>, layer: usize, layout: usize) -> Result<Arc<Graph>, String> {
        let mut graphs = self.graphs.lock().map_err(|_| "navmesh DSM graph owner is poisoned")?;
        if let Some(graph) = graphs.get(&(layer, layout)).and_then(std::sync::Weak::upgrade) { return Ok(graph); }
        let layer_index = layer;
        let layer = self.layers.get(layer).ok_or("navmesh layer index is invalid")?;
        let text = std::str::from_utf8(self.bytes(&layer.dsm_info)).map_err(|error| error.to_string())?;
        let root = Value { text, start: layer.dsm_info.start };
        let layouts = root.required("m_dsmLayouts")?;
        let count: usize = layouts.required("num")?.number()?;
        if count > 2 || layout >= count { return Err("DSM static layout index is invalid".into()); }
        let value = layouts.required(&format!("item[{layout}]"))?;
        let source_cells = value.required("m_cells")?;
        let count: usize = source_cells.required("num")?.number()?;
        if count > source_cells.text.len() { return Err("DSM cell count exceeds source".into()); }
        let mut cells = Vec::with_capacity(count);
        source_cells.fields(|name, cell| {
            if name == "num" { return Ok(()); }
            if name.strip_prefix("item[").and_then(|name| name.strip_suffix(']')).and_then(|name| name.parse::<usize>().ok()) != Some(cells.len()) {
                return Err("DSM cells are not in serialized order".into());
            }
            let key = cell.required("m_key")?;
            let area = cell.field("m_areaVolume")?.filter(|area| area.text != "NULL");
            cells.push(Cell { key: [key.required("m_x")?.number()?, key.required("m_y")?.number()?, key.required("m_level")?.number()?],
                nav_face_id: cell.required("m_navFaceId")?.number()?, offset: cell.required("m_offset")?.number()?,
                area_volume: area.map(|area| Ok::<_, String>((u32::try_from(area.start).map_err(|_| "DSM source offset exceeds32bits")?,
                    u32::try_from(area.start + area.text.len()).map_err(|_| "DSM source offset exceeds32bits")?))).transpose()? });
            Ok(())
        })?;
        if cells.len() != count { return Err("DSM serialized cell count differs".into()); }
        let connectivity = value.required("m_connectivities")?;
        let connectivity_count: usize = connectivity.required("num")?.number()?;
        if connectivity_count != count { return Err("DSM connectivity and cell counts differ".into()); }
        let mut neighbor_offsets = Vec::with_capacity(count + 1);
        let mut neighbors = Vec::new();
        neighbor_offsets.push(0);
        connectivity.fields(|name, item| {
            if name == "num" { return Ok(()); }
            if name.strip_prefix("item[").and_then(|name| name.strip_suffix(']')).and_then(|name| name.parse::<usize>().ok()) != Some(neighbor_offsets.len() - 1) {
                return Err("DSM connectivities are not in serialized order".into());
            }
            let connections = item.required("m_connections")?;
            let degree: usize = connections.required("num")?.number()?;
            if degree > 16 { return Err("DSM connection count exceeds source static16 capacity".into()); }
            let start = neighbors.len();
            connections.fields(|name, connection| {
                if name == "num" { return Ok(()); }
                if name.strip_prefix("item[").and_then(|name| name.strip_suffix(']')).and_then(|name| name.parse::<usize>().ok()) != Some(neighbors.len() - start) {
                    return Err("DSM neighbor entries are not in serialized order".into());
                }
                let index: u16 = connection.required("m_neighborCellIndex")?.number()?;
                if usize::from(index) >= count { return Err("DSM neighbor index exceeds actual cell count".into()); }
                neighbors.push(index);
                Ok(())
            })?;
            if neighbors.len() - start != degree { return Err("DSM neighbor count differs".into()); }
            neighbor_offsets.push(u32::try_from(neighbors.len()).map_err(|_| "DSM neighbor table exceeds32bits")?);
            Ok(())
        })?;
        if neighbor_offsets.len() != count + 1 { return Err("DSM connectivity record count differs".into()); }
        let origin = value.required("m_gridOrigin")?;
        let grid_origin = [origin.required("x")?.number()?, origin.required("y")?.number()?, origin.required("z")?.number()?];
        let graph = Arc::new(Graph { source: self.clone(), range: value.start..value.start + value.text.len(),
            cells: cells.into(), grid_origin, nav_mesh_id: (u32::from(layer.world) << 16) | u32::from(layer.instance),
            neighbor_offsets: neighbor_offsets.into(), neighbors: neighbors.into() });
        graphs.insert((layer_index, layout), Arc::downgrade(&graph));
        Ok(graph)
    }
}
