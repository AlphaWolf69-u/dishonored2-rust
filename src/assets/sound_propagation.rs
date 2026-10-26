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
fn u64le(reader: &mut Cursor<&[u8]>) -> io::Result<u64> {
    Ok(u64::from_le_bytes(raw(reader)?))
}
fn take<'a>(reader: &mut Cursor<&'a [u8]>, length: u64) -> io::Result<&'a [u8]> {
    let start =
        usize::try_from(reader.position()).map_err(|_| invalid("acoustic offset overflow"))?;
    let length = usize::try_from(length).map_err(|_| invalid("acoustic length overflow"))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| invalid("acoustic offset overflow"))?;
    let bytes = reader
        .get_ref()
        .get(start..end)
        .ok_or_else(|| invalid("truncated acoustic resource"))?;
    reader.set_position(end as u64);
    Ok(bytes)
}

impl Propagation {
    pub fn parse(bytes: &[u8]) -> io::Result<Self> {
        let mut reader = Cursor::new(bytes);
        let version = u64le(&mut reader)?;
        if version != 13 {
            return Err(invalid(format!(
                "unsupported sound propagation version {version}"
            )));
        }
        let count = u32le(&mut reader)? as usize;
        if count > bytes.len() / 30 {
            return Err(invalid("invalid acoustic zone count"));
        }
        let mut zones = Vec::with_capacity(count);
        for index in 0..count {
            // 1400b9eb0 allocates this many portal slots; portal references
            // themselves are rebuilt from the portal endpoints after loading.
            let portal_capacity = u32le(&mut reader)?;
            let id = u16le(&mut reader)?;
            let length = u32le(&mut reader)?;
            let name = take(&mut reader, u64::from(length))?;
            let name = name.split(|&byte| byte == 0).next().unwrap_or_default();
            let auxiliary_bus = std::str::from_utf8(name)
                .map_err(|error| invalid(error.to_string()))?
                .to_owned();
            let properties = [
                u32le(&mut reader)?,
                u32le(&mut reader)?,
                u32le(&mut reader)?,
            ];
            let length = u64le(&mut reader)?;
            let shape = take(&mut reader, length)?;
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
        let count = u32le(&mut reader)? as usize;
        if count > (bytes.len() - reader.position() as usize) / 116 {
            return Err(invalid("invalid acoustic portal count"));
        }
        let mut portals = Vec::with_capacity(count);
        for _ in 0..count {
            let mut geometry = [0; 27];
            for value in &mut geometry {
                *value = u32le(&mut reader)?;
            }
            let endpoints = [u16le(&mut reader)?, u16le(&mut reader)?];
            let properties = [u16le(&mut reader)?, u16le(&mut reader)?];
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
        let zone_count = u16le(&mut reader)?;
        let length = u64le(&mut reader)?;
        let routes = Routes {
            zone_count,
            bytes: take(&mut reader, length)?.to_vec(),
        };
        let count = usize::from(zone_count);
        if routes.bytes.len() < count * (count + 1) / 2 * 4 {
            return Err(invalid("truncated acoustic route offsets"));
        }
        if zones.iter().any(|zone| zone.id >= zone_count) {
            return Err(invalid("acoustic zone ID exceeds route table"));
        }
        if reader.position() != bytes.len() as u64 {
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
