//! Original CPU topology reduction and local-query worker ownership.

#[derive(Debug,Clone,Default,PartialEq,Eq)]
pub struct NativeCpuTopology {
    pub words:[u32;16],
}

///0de070 reduces32-byte GetLogicalProcessorInformation records. Preserve
///the original cache-count behavior: a zero count remains zero while the
///descriptor is replaced; an already positive count is incremented.
pub fn reduce_cpu_topology(records:&[u8])->Result<NativeCpuTopology,String> {
    let mut result=NativeCpuTopology::default();
    for record in records.chunks_exact(32) {
        let word=|at|u32::from_le_bytes(record[at..at+4].try_into().unwrap());
        match word(8) {
            0=> {
                result.words[1]=result.words[1].wrapping_add(1);
                result.words[2]=result.words[2].wrapping_add(u64::from_le_bytes(record[..8].try_into().unwrap()).count_ones());
            },
            1=>result.words[3]=result.words[3].wrapping_add(1),
            2=> {
                let level=record[16].wrapping_sub(1);
                if level<3 {
                    let at=usize::from(level)*4+4;
                    if (result.words[at] as i32)>0 {result.words[at]=result.words[at].wrapping_add(1);}
                    else {
                        result.words[at+1]=u32::from(record[17]);
                        result.words[at+2]=u32::from(u16::from_le_bytes(record[18..20].try_into().unwrap()));
                        result.words[at+3]=word(20);
                    }
                }
            },
            3=>result.words[0]=result.words[0].wrapping_add(1),
            value=>return Err(format!("unsupported native LOGICAL_PROCESSOR_RELATIONSHIP {value}")),
        }
    }
    Ok(result)
}

///079c60 chooses physical cores minus one, clamped to2..15 workers.
///07a910 includes the caller;37ef90 adds its local-query job and caps at16.
pub fn local_shadow_query_job_count(topology:&NativeCpuTopology)->u32 {
    let workers=(topology.words[1] as i32).wrapping_sub(1).clamp(2,15);
    (workers+2).min(16) as u32
}

#[cfg(target_os="windows")]
pub fn cpu_topology()->Result<NativeCpuTopology,String> {
    #[link(name="kernel32")]
    unsafe extern "system" {
        fn GetLogicalProcessorInformation(buffer:*mut std::ffi::c_void,length:*mut u32)->i32;
    }
    let mut storage=Vec::<u64>::new();let mut length=0u32;
    loop {
        let pointer=if storage.is_empty() {std::ptr::null_mut()}else{storage.as_mut_ptr().cast()};
        if unsafe{GetLogicalProcessorInformation(pointer,&mut length)}!=0 {
            if length as usize>storage.len()*8 {return Err("native CPU topology exceeds its supplied buffer".into());}
            let bytes=unsafe{std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(),length as usize)};
            return reduce_cpu_topology(bytes);
        }
        let error=std::io::Error::last_os_error();
        if error.raw_os_error()!=Some(122) {return Err(format!("GetLogicalProcessorInformation: {error}"));}
        storage.resize((length as usize+7)/8,0);
    }
}

#[cfg(not(target_os="windows"))]
pub fn cpu_topology()->Result<NativeCpuTopology,String> {
    Err("native CPU topology requires Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn topology_reduction_matches_original_instructions() {
        for (case,line) in include_str!("../analysis/original_cpu_topology_cases.txt").lines().enumerate() {
            let words=line.split_whitespace().map(|v|u32::from_str_radix(v,16).unwrap()).collect::<Vec<_>>();
            let count=words[0] as usize;let records=words[1..1+count*8].iter().flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>();
            assert_eq!(reduce_cpu_topology(&records).unwrap().words,words[1+count*8..],"native topology case{case}");
        }
    }
}
