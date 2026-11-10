//! Version-13 acoustic volumes and portal routes (1400b7fe0).
use super::physworld::{self, query_geometry::QueryShapes};
use std::io::{self, Cursor, Read};

#[derive(Debug)]
pub struct Zone {
    pub id: u16,
    pub portal_capacity: u32,
    pub auxiliary_bus: String,
    pub properties: [u32; 3],
    pub geometry: QueryShapes,
}

#[derive(Debug)]
pub struct Portal {
    /// Serialized geometry; interpretation belongs to the path solver.
    pub geometry: [u32; 27],
    pub zones: [u16; 2],
    pub properties: [u16; 2],
}

#[derive(Debug)]
pub struct Propagation {
    pub zones: Vec<Zone>,
    pub portals: Vec<Portal>,
    pub routes: Routes,
}

#[derive(Debug)]
pub struct Routes {
    pub zone_count: u16,
    bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct Route {
    pub cost_millimetres: u32,
    pub portals: Vec<u16>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn raw<const N: usize>(reader: &mut Cursor<&[u8]>) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn u16le(reader: &mut Cursor<&[u8]>) -> io::Result<u16> {
    Ok(u16::from_le_bytes(raw(reader)?))
}
fn u32le(reader: &mut Cursor<&[u8]>) -> io::Result<u32> {
    Ok(u32::from_le_bytes(raw(reader)?))
}
enum Source<'a> {
    Slice(&'a [u8]),
    Stream { reader: &'a mut dyn Read, scratch: Vec<u8> },
}

struct Input<'a> {
    source: Source<'a>,
    remaining: usize,
}

impl Input<'_> {
    fn consume(&mut self, length: u64) -> io::Result<usize> {
        let length = usize::try_from(length).map_err(|_| invalid("acoustic length overflow"))?;
        self.remaining = self.remaining.checked_sub(length)
            .ok_or_else(|| invalid("truncated acoustic resource"))?;
        Ok(length)
    }

    fn bytes(&mut self, length: u64) -> io::Result<&[u8]> {
        let length = self.consume(length)?;
        match &mut self.source {
            Source::Slice(bytes) => Ok(split_bytes(bytes, length)),
            Source::Stream { reader, scratch } => {
                if scratch.len() < length { scratch.resize(length, 0); }
                let bytes = &mut scratch[..length];
                reader.read_exact(bytes)?;
                Ok(bytes)
            }
        }
    }

    fn route_bytes(&mut self, length: u64) -> io::Result<Vec<u8>> {
        let length = self.consume(length)?;
        match &mut self.source {
            Source::Slice(bytes) => Ok(split_bytes(bytes, length).to_vec()),
            Source::Stream { reader, .. } => {
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes)?;
                Ok(bytes)
            }
        }
    }

    fn u16(&mut self) -> io::Result<u16> { Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap())) }
    fn u32(&mut self) -> io::Result<u32> { Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap())) }
    fn u64(&mut self) -> io::Result<u64> { Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap())) }
}

fn split_bytes<'a>(bytes: &mut &'a [u8], length: usize) -> &'a [u8] {
    let (head, tail) = bytes.split_at(length);
    *bytes = tail;
    head
}

impl Propagation {
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        Self::read(&mut Input { source: Source::Slice(bytes), remaining: bytes.len() })
    }

    pub fn load(catalog: &super::archive::Catalog, path: &str) -> Result<Option<Self>, String> {
        let trace = std::env::var_os("DH2_DEBUG_ACOUSTIC_STAGING").is_some_and(|v| v == "1");
        let buffered = std::env::var_os("DH2_DEBUG_BUFFER_ACOUSTICS").is_some_and(|v| v == "1");
        // The archive reader allocates a 64 KiB buffer. Small members are
        // cheaper to borrow whole, including menu and Void acoustic files.
        if !buffered && catalog.resource_size(path).is_some_and(|bytes| bytes > 2 * 65_536) {
            return catalog.with_stream(path, |reader, size| {
                let mut input = Input { source: Source::Stream { reader, scratch: Vec::new() }, remaining: size };
                let graph = Self::read(&mut input).map_err(|error| error.to_string())?;
                if trace {
                    let Source::Stream { scratch, .. } = &input.source else { unreachable!() };
                    graph.report_staging(path, size, 65_536 + scratch.capacity(), true);
                }
                Ok(graph)
            });
        }
        let Some(bytes) = catalog.read(path).map_err(|error| error.to_string())? else { return Ok(None) };
        let graph = Self::parse(&bytes).map_err(|error| error.to_string())?;
        if trace { graph.report_staging(path, bytes.len(), bytes.capacity(), false); }
        Ok(Some(graph))
    }

    fn report_staging(&self, path: &str, member_bytes: usize, staging_bytes: usize, streamed: bool) {
        let mut digest = blake3::Hasher::new();
        for zone in &self.zones { digest.update(format!("{zone:?}").as_bytes()); }
        for portal in &self.portals { digest.update(format!("{portal:?}").as_bytes()); }
        digest.update(&self.routes.zone_count.to_le_bytes());
        digest.update(&self.routes.bytes);
        eprintln!("ACOUSTIC_STAGING path={path} streamed={streamed} member_bytes={member_bytes} staging_bytes={staging_bytes} route_bytes={} signature={}",
            self.routes.bytes.len(), digest.finalize());
    }

    fn read(reader: &mut Input<'_>) -> io::Result<Self> {
        let version = reader.u64()?;
        if version != 13 {
            return Err(invalid(format!(
                "unsupported sound propagation version {version}"
            )));
        }
        let count = reader.u32()? as usize;
        if count > reader.remaining / 30 {
            return Err(invalid("invalid acoustic zone count"));
        }
        let mut zones = Vec::with_capacity(count);
        for index in 0..count {
            // 1400b9eb0 allocates this many portal slots; portal references
            // themselves are rebuilt from the portal endpoints after loading.
            let portal_capacity = reader.u32()?;
            let id = reader.u16()?;
            let length = reader.u32()?;
            let name = reader.bytes(u64::from(length))?;
            let name = name.split(|&byte| byte == 0).next().unwrap_or_default();
            let auxiliary_bus = std::str::from_utf8(name)
                .map_err(|error| invalid(error.to_string()))?
                .to_owned();
            let properties = [
                reader.u32()?,
                reader.u32()?,
                reader.u32()?,
            ];
            let length = reader.u64()?;
            let shape = reader.bytes(length)?;
            let geometry = physworld::parse_embedded_havok(shape)
                .and_then(|world| world.query_shapes(shape, 1))
                .map_err(|error| invalid(format!("acoustic zone {index} (ID {id}): {error}")))?;
            zones.push(Zone {
                id,
                portal_capacity,
                auxiliary_bus,
                properties,
                geometry,
            });
        }
        let count = reader.u32()? as usize;
        if count > reader.remaining / 116 {
            return Err(invalid("invalid acoustic portal count"));
        }
        let mut portals = Vec::with_capacity(count);
        for _ in 0..count {
            let record = reader.bytes(116)?;
            let mut record = Cursor::new(record);
            let mut geometry = [0; 27];
            for value in &mut geometry { *value = u32le(&mut record)?; }
            let endpoints = [u16le(&mut record)?, u16le(&mut record)?];
            let properties = [u16le(&mut record)?, u16le(&mut record)?];
            if endpoints
                .iter()
                .any(|&zone| usize::from(zone) >= zones.len())
            {
                return Err(invalid("acoustic portal references missing zone"));
            }
            portals.push(Portal {
                geometry,
                zones: endpoints,
                properties,
            });
        }
        let zone_count = reader.u16()?;
        let length = reader.u64()?;
        let routes = Routes {
            zone_count,
            bytes: reader.route_bytes(length)?,
        };
        let count = usize::from(zone_count);
        if routes.bytes.len() < count * (count + 1) / 2 * 4 {
            return Err(invalid("truncated acoustic route offsets"));
        }
        if zones.iter().any(|zone| zone.id >= zone_count) {
            return Err(invalid("acoustic zone ID exceeds route table"));
        }
        if reader.remaining != 0 {
            return Err(invalid("trailing acoustic resource bytes"));
        }
        Ok(Self {
            zones,
            portals,
            routes,
        })
    }
}

impl Routes {
    /// 1400b7790/1400b7ce0: symmetric triangular lookup. Portal order is
    /// authored from lower to higher zone ID; the caller reverses it when
    /// traversing from the higher ID toward the lower ID.
    pub fn between(&self, first: u16, second: u16) -> io::Result<Vec<Route>> {
        let count = usize::from(self.zone_count);
        let low = usize::from(first.min(second));
        let high = usize::from(first.max(second));
        if high >= count {
            return Err(invalid("acoustic route zone ID out of bounds"));
        }
        let index = high + (count - 1) * low - low * low.saturating_sub(1) / 2;
        let mut reader = Cursor::new(self.bytes.as_slice());
        reader.set_position((index * 4) as u64);
        let offset = u32le(&mut reader)?;
        if offset == 0 {
            return Ok(Vec::new());
        }
        reader.set_position(u64::from(offset));
        let count = u16le(&mut reader)?;
        let mut routes = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let header = u32le(&mut reader)?;
            let mut portals = Vec::with_capacity((header >> 24) as usize);
            for _ in 0..header >> 24 {
                portals.push(u16le(&mut reader)?);
            }
            routes.push(Route {
                cost_millimetres: header & 0x00ff_ffff,
                portals,
            });
        }
        Ok(routes)
    }
}
