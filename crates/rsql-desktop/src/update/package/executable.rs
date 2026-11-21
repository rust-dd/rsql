use object::{LittleEndian, macho::*, read::macho::MachHeader};
use rsql_core::update::Artifact;
use std::io::Read;

pub(super) fn inspect(
    reader: &mut impl Read,
    size: u64,
    artifact: &Artifact,
) -> Result<(), String> {
    type Header = MachHeader64<LittleEndian>;
    let mut bytes = vec![0; std::mem::size_of::<Header>()];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| "Missing Mach-O header")?;
    let hdr = Header::parse(bytes.as_slice(), 0).map_err(|_| "Expected thin 64-bit Mach-O")?;
    let (cpu, subtype) = match artifact.target.as_str() {
        "aarch64-apple-darwin" => (CPU_TYPE_ARM64, CPU_SUBTYPE_ARM64_ALL),
        "x86_64-apple-darwin" => (CPU_TYPE_X86_64, CPU_SUBTYPE_X86_64_ALL),
        _ => return Err("Unsupported executable architecture".into()),
    };
    let en = LittleEndian;
    let cmds = hdr.sizeofcmds(en) as usize;
    if !hdr.is_little_endian()
        || hdr.filetype(en) != MH_EXECUTE
        || hdr.cputype(en) != cpu
        || hdr.cpusubtype(en) != subtype
        || cmds > super::HEADER_LIMIT
        || hdr.ncmds(en) > 4096
    {
        return Err("Mach-O target, file type or command size mismatch".into());
    }
    let prefix = bytes.len();
    bytes.resize(prefix + cmds, 0);
    reader
        .read_exact(&mut bytes[prefix..])
        .map_err(|_| "Truncated Mach-O commands")?;
    let hdr = Header::parse(bytes.as_slice(), 0).map_err(|_| "Invalid Mach-O header")?;
    let table = hdr
        .load_commands(en, bytes.as_slice(), 0)
        .map_err(|_| "Invalid Mach-O commands")?;
    let mut total = 0;
    let mut text = None;
    let mut entry = None;
    let mut os = None;
    for cmd in table {
        let cmd = cmd.map_err(|_| "Malformed Mach-O command")?;
        total += cmd.cmdsize() as usize;
        if let Some((seg, _)) = cmd.segment_64().map_err(|_| "Invalid Mach-O segment")? {
            let start = seg.fileoff.get(en);
            let end = start
                .checked_add(seg.filesize.get(en))
                .filter(|e| *e <= size)
                .ok_or("Invalid segment range")?;
            if seg.segname.strip_suffix(&[0; 10]) == Some(b"__TEXT")
                && seg.initprot.get(en) & 4 != 0
                && text.replace(start..end).is_some()
            {
                return Err("Duplicate text segment".into());
            }
        }
        if let Some(main) = cmd.entry_point().map_err(|_| "Invalid Mach-O entry")?
            && entry.replace(main.entryoff.get(en)).is_some()
        {
            return Err("Duplicate executable entry".into());
        }
        if let Some(build) = cmd
            .build_version()
            .map_err(|_| "Invalid Mach-O build metadata")?
            && (build.platform.get(en) != PLATFORM_MACOS
                || os.replace(build.minos.get(en)).is_some())
        {
            return Err("Mach-O platform mismatch".into());
        }
    }
    let min = os.ok_or("Missing Mach-O deployment target")?;
    let floor = semver::Version::new(
        (min >> 16).into(),
        ((min >> 8) & 255).into(),
        (min & 255).into(),
    );
    if total != cmds
        || floor != artifact.minimum_os
        || !entry
            .zip(text)
            .is_some_and(|(offset, range)| offset >= bytes.len() as u64 && range.contains(&offset))
    {
        return Err("Mach-O deployment target or entry point mismatch".into());
    }
    Ok(())
}
