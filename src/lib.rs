#![no_std]
#![crate_name = "elfloader"]
#![crate_type = "lib"]

#[cfg(test)]
#[macro_use]
extern crate std;
#[cfg(test)]
extern crate env_logger;

extern crate log;
extern crate xmas_elf;

use log::*;

use core::fmt;

use xmas_elf::dynamic::Tag;
use xmas_elf::header;
use xmas_elf::program::ProgramHeader::{Ph32, Ph64};
use xmas_elf::program::{ProgramHeader64, ProgramIter, SegmentData, Type};
use xmas_elf::sections::SectionData;
use xmas_elf::*;

pub use xmas_elf::program::Flags;
pub use xmas_elf::sections::Rela;
pub use xmas_elf::symbol_table::{Entry, Entry64};
pub use xmas_elf::{P32, P64};

pub type PAddr = u64;
pub type VAddr = u64;

// Should be in xmas-elf see: https://github.com/nrc/xmas-elf/issues/54
#[derive(Eq, PartialEq, Debug, Clone, Copy)]
#[allow(non_camel_case_types)]
#[repr(u32)]
pub enum TypeRela64 {
    /// No relocation.
    R_NONE,
    /// Add 64 bit symbol value.
    R_64,
    /// PC-relative 32 bit signed sym value.
    R_PC32,
    /// PC-relative 32 bit GOT offset.
    R_GOT32,
    /// PC-relative 32 bit PLT offset.
    R_PLT32,
    /// Copy data from shared object.
    R_COPY,
    /// Set GOT entry to data address.
    R_GLOB_DAT,
    /// Set GOT entry to code address.
    R_JMP_SLOT,
    /// Add load address of shared object.
    R_RELATIVE,
    /// Add 32 bit signed pcrel offset to GOT.
    R_GOTPCREL,
    /// Add 32 bit zero extended symbol value
    R_32,
    /// Add 32 bit sign extended symbol value
    R_32S,
    /// Add 16 bit zero extended symbol value
    R_16,
    /// Add 16 bit signed extended pc relative symbol value
    R_PC16,
    /// Add 8 bit zero extended symbol value
    R_8,
    /// Add 8 bit signed extended pc relative symbol value
    R_PC8,
    /// ID of module containing symbol
    R_DTPMOD64,
    /// Offset in TLS block
    R_DTPOFF64,
    /// Offset in static TLS block
    R_TPOFF64,
    /// PC relative offset to GD GOT entry
    R_TLSGD,
    /// PC relative offset to LD GOT entry
    R_TLSLD,
    /// Offset in TLS block
    R_DTPOFF32,
    /// PC relative offset to IE GOT entry
    R_GOTTPOFF,
    /// Offset in static TLS block
    R_TPOFF32,
    /// Unkown
    Unknown(u32),
}

impl TypeRela64 {
    // Construt a new TypeRela64
    pub fn from(typ: u32) -> TypeRela64 {
        use TypeRela64::*;
        match typ {
            0 => R_NONE,
            1 => R_64,
            2 => R_PC32,
            3 => R_GOT32,
            4 => R_PLT32,
            5 => R_COPY,
            6 => R_GLOB_DAT,
            7 => R_JMP_SLOT,
            8 => R_RELATIVE,
            9 => R_GOTPCREL,
            10 => R_32,
            11 => R_32S,
            12 => R_16,
            13 => R_PC16,
            14 => R_8,
            15 => R_PC8,
            16 => R_DTPMOD64,
            17 => R_DTPOFF64,
            18 => R_TPOFF64,
            19 => R_TLSGD,
            20 => R_TLSLD,
            21 => R_DTPOFF32,
            22 => R_GOTTPOFF,
            23 => R_TPOFF32,
            x => Unknown(x),
        }
    }
}

/// Abstract representation of a loadable ELF binary.
pub struct ElfBinary<'s> {
    name: &'s str,
    file: ElfFile<'s>,
}

impl<'s> fmt::Debug for ElfBinary<'s> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ElfBinary{{ {} [", self.name)?;
        for p in self.program_headers() {
            write!(f, " pheader = {}", p)?;
        }
        write!(f, "] }}")
    }
}

/// Implement this for ELF loading.
pub trait ElfLoader {
    /// Allocates a virtual region of size amount of bytes.
    fn allocate(&mut self, base: VAddr, size: usize, flags: Flags) -> Result<(), &'static str>;

    /// Copies the region into the base.
    fn load(&mut self, base: VAddr, region: &[u8]) -> Result<(), &'static str>;

    /// Relocate `entry` within the loaded ELF `header.
    fn relocate(&mut self, entry: &Rela<P64>) -> Result<(), &'static str>;
}

impl<'s> ElfBinary<'s> {
    /// Create a new ElfBinary.
    /// Makes sure that the provided region has valid ELF magic byte sequence
    /// and is big enough to contain at least the ELF file header
    /// otherwise it will return None.
    pub fn new(name: &'s str, region: &'s [u8]) -> Result<ElfBinary<'s>, &'static str> {
        let elf_file = ElfFile::new(region)?;
        Ok(ElfBinary {
            name,
            file: elf_file,
        })
    }

    /// Return the entry point of the ELF file.
    ///
    /// Note this may be zero in case of position independent executables.
    pub fn entry_point(&self) -> u64 {
        self.file.header.pt2.entry_point()
    }

    /// Create a slice of the program headers.
    pub fn program_headers(&self) -> ProgramIter {
        self.file.program_iter()
    }

    /// Get the name of the sectione
    pub fn symbol_name(&self, symbol: &'s Entry) -> &'s str {
        symbol.get_name(&self.file).unwrap_or("unknown")
    }

    /// Enumerate all the symbols in the file
    pub fn for_each_symbol<F: FnMut(&'s Entry)>(&self, mut func: F) -> Result<(), &'static str> {
        let symbol_section = self
            .file
            .find_section_by_name(".symtab")
            .ok_or("No .symtab section")?;
        let symbol_table = symbol_section.get_data(&self.file)?;
        if let SectionData::SymbolTable64(entries) = symbol_table {
            for entry in entries {
                trace!("entry {:?}", entry);
                func(entry);
            }
            Ok(())
        } else {
            Err(".symtab does not contain a symbol table")
        }
    }

    /// Can we load this binary on our platform?
    fn is_loadable(&self) -> Result<(), &'static str> {
        let header = self.file.header;
        let typ = header.pt2.type_().as_type();

        if header.pt1.class() != header::Class::SixtyFour {
            Err("Not 64bit ELF")
        } else if header.pt1.version() != header::Version::Current {
            Err("Invalid version")
        } else if header.pt1.data() != header::Data::LittleEndian {
            Err("Wrong Endianness")
        } else if !(header.pt1.os_abi() == header::OsAbi::SystemV
            || header.pt1.os_abi() == header::OsAbi::Linux)
        {
            Err("Wrong ABI")
        } else if !(typ == header::Type::Executable || typ == header::Type::SharedObject) {
            error!("Invalid ELF type {:?}", typ);
            Err("Invalid ELF type")
        } else if header.pt2.machine().as_machine() != header::Machine::X86_64 {
            Err("ELF file is not for x86-64 machine")
        } else {
            Ok(())
        }
    }

    /// Process the relocation entries for a given program header `loaded_header`
    /// issues call to `loader.relocate` and passes the relocation entry.
    ///
    /// TODO: This currently processes all relocation entries rather than only
    /// those for `loaded_header`?
    fn maybe_relocate(&self, loader: &mut ElfLoader) -> Result<(), &'static str> {
        // It's easier to just locate the section by name:
        let rela_section_dyn = self
            .file
            .find_section_by_name(".rela.dyn")
            .ok_or("No .rela.dyn section")?;

        let data = rela_section_dyn.get_data(&self.file)?;
        if let SectionData::Rela64(rela_entries) = data {
            // Now we finally have a list of relocation we're supposed to perform:
            for entry in rela_entries {
                let _typ = TypeRela64::from(entry.get_type());
                // Does the entry blong to the current header?
                loader.relocate(entry)?;
            }

            Ok(())
        } else {
            return Err("Unexpected Section Data: was not Rela64");
        }
    }

    /// Processes a dynamic header section.
    ///
    /// This section contains mostly entry point to other section headers (like relocation).
    /// At the moment this just does sanity checking for relocation later.
    fn check_dynamic(
        &self,
        p: &ProgramHeader64,
        _loader: &mut ElfLoader,
    ) -> Result<(), &'static str> {
        info!("load dynamic segement {:?}", p);

        // Walk through the dynamic program header and find the rela and sym_tab section offsets:
        let segment = p.get_data(&self.file)?;
        let mut rela = 0;
        let mut rela_size = 0;
        match segment {
            SegmentData::Dynamic64(dyn_entries) => {
                for dyn_entry in dyn_entries {
                    let tag = dyn_entry.get_tag()?;
                    match tag {
                        Tag::Rela => rela = dyn_entry.get_ptr()?,
                        Tag::RelaSize => rela_size = dyn_entry.get_val()?,
                        _ => trace!("unsupported {:?}", dyn_entry),
                    }
                }
            }
            _ => {
                return Err("Segment for dynamic data was not Dynamic64?");
            }
        };
        trace!("rela size {:?} rela off {:?}", rela_size, rela);

        // It's easier to just locate the section by name:
        let rela_section_dyn = self
            .file
            .find_section_by_name(".rela.dyn")
            .ok_or("No .rela.dyn section")?;

        // For sanity we still check it's size is the same as reported in DYNAMIC
        if rela_size != rela_section_dyn.size() || rela != rela_section_dyn.offset() {
            return Err("Dynamic offset/size doesn't match with .rela.dyn entries");
        }

        Ok(())
    }

    /// Processing the program headers and issue commands to loader.
    ///
    /// Will tell loader to create space in the address space / region where the
    /// header is supposed to go, then copy it there, and finally relocate it.
    pub fn load(&self, loader: &mut ElfLoader) -> Result<(), &'static str> {
        self.is_loadable()?;

        // Allocate all headers
        for p in self.file.program_iter() {
            match p {
                Ph32(_) => {
                    error!("Encountered 32-bit header in 64bit ELF?");
                    return Err("Encountered 32-bit header");
                }
                Ph64(header) => {
                    let typ = header.get_type()?;
                    if typ == Type::Load {
                        loader.allocate(
                            header.virtual_addr,
                            header.mem_size as usize,
                            header.flags,
                        )?;
                    } else if typ == Type::Dynamic {
                        self.check_dynamic(header, loader)?;
                    }
                }
            }
        }

        // Load all headers
        for p in self.file.program_iter() {
            if let Ph64(header) = p {
                let typ = header.get_type()?;
                if typ == Type::Load {
                    loader.load(header.virtual_addr, header.raw_data(&self.file))?;
                }
            }
        }

        // Relocate headers
        self.maybe_relocate(loader)?;

        Ok(())
    }
}

#[cfg(test)]
mod test {

    use crate::*;

    use std::fs;

    use std::vec::Vec;

    #[derive(Eq, Clone, PartialEq, Copy, Debug)]
    enum LoaderAction {
        Allocate(VAddr, usize, Flags),
        Load(VAddr, usize),
        Relocate(VAddr, u64),
    }
    struct TestLoader {
        vbase: VAddr,
        actions: Vec<LoaderAction>,
    }

    impl TestLoader {
        fn new(offset: VAddr) -> TestLoader {
            TestLoader {
                vbase: offset,
                actions: Vec::with_capacity(12),
            }
        }
    }

    impl ElfLoader for TestLoader {
        fn allocate(&mut self, base: VAddr, size: usize, flags: Flags) -> Result<(), &'static str> {
            info!(
                "allocate base = {:#x} size = {:#x} flags = {}",
                base, size, flags
            );
            self.actions.push(LoaderAction::Allocate(base, size, flags));
            Ok(())
        }

        fn relocate(&mut self, entry: &Rela<P64>) -> Result<(), &'static str> {
            let typ = TypeRela64::from(entry.get_type());

            // Get the pointer to where the relocation happens in the
            // memory where we loaded the headers
            //
            // vbase is the new base where we locate the binary
            //
            // get_offset(): For an executable or shared object, the value indicates
            // the virtual address of the storage unit affected by the relocation.
            // This information makes the relocation entries more useful for the runtime linker.
            let addr: *mut u64 = (self.vbase + entry.get_offset()) as *mut u64;

            match typ {
                TypeRela64::R_64 => {
                    trace!("R_64");
                    Ok(())
                }
                TypeRela64::R_RELATIVE => {
                    // This is a relative relocation, add the offset (where we put our
                    // binary in the vspace) to the addend and we're done.
                    self.actions.push(LoaderAction::Relocate(
                        addr as u64,
                        self.vbase + entry.get_addend(),
                    ));
                    trace!(
                        "R_RELATIVE *{:p} = {:#x}",
                        addr,
                        self.vbase + entry.get_addend()
                    );
                    Ok(())
                }
                TypeRela64::R_GLOB_DAT => {
                    trace!("TypeRela64::R_GLOB_DAT: Can't handle that.");
                    Ok(())
                }
                TypeRela64::R_NONE => Ok(()),
                _ => Err("Unexpected relocation encountered"),
            }
        }

        fn load(&mut self, base: VAddr, region: &[u8]) -> Result<(), &'static str> {
            info!("load base = {:#x} size = {:#x} region", base, region.len());
            self.actions.push(LoaderAction::Load(base, region.len()));

            Ok(())
        }
    }

    fn init() {
        let _ = env_logger::builder().is_test(true).try_init();
    }

    #[test]
    fn load_pie_elf() {
        init();
        let binary_blob = fs::read("test/test").expect("Can't read binary");
        let binary = ElfBinary::new("test", binary_blob.as_slice()).expect("Got proper ELF file");

        let mut loader = TestLoader::new(0x1000_0000);
        binary.load(&mut loader).expect("Can't load?");

        assert!(loader
            .actions
            .iter()
            .find(|&&x| x == LoaderAction::Allocate(VAddr::from(0x0u64), 0x888, Flags(1 | 4)))
            .is_some());
        assert!(loader
            .actions
            .iter()
            .find(|&&x| x == LoaderAction::Allocate(VAddr::from(0x200db8u64), 0x260, Flags(2 | 4)))
            .is_some());
        assert!(loader
            .actions
            .iter()
            .find(|&&x| x == LoaderAction::Load(VAddr::from(0x0u64), 0x888))
            .is_some());
        assert!(loader
            .actions
            .iter()
            .find(|&&x| x == LoaderAction::Load(VAddr::from(0x200db8u64), 0x258))
            .is_some());
        assert!(loader
            .actions
            .iter()
            .find(|&&x| x == LoaderAction::Relocate(0x1000_0000 + 0x200db8, 0x1000_0000 + 0x000640))
            .is_some());
        assert!(loader
            .actions
            .iter()
            .find(|&&x| x == LoaderAction::Relocate(0x1000_0000 + 0x200dc0, 0x1000_0000 + 0x000600))
            .is_some());

        //info!("test {:#?}", loader.actions);
    }
}
