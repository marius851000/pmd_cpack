#![allow(clippy::cast_lossless)]
use std::fmt;
use std::error::Error;
use std::io;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use io_partition::PartitionMutex;

#[derive(Debug)]
/// Possible error that may happen with CPack
pub enum CPackError {
    IOError(io::Error),
    PoisonedLock,
    FourFirstByteNotZero([u8; 4]),
    EndOfFileOutOfScope(u32, u32, u32),
    EndOfHeaderNotZero(u64, [u8; 8]),
    PartitionCreationError(io::Error),
}

impl Error for CPackError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::IOError(err) | Self::PartitionCreationError(err) => {
                Some(err)
            },
            _ => None,
        }
    }
}
impl fmt::Display for CPackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CPackError::IOError(_) => write!(f, "an error happened while performing an IO on the input file"),
            CPackError::PoisonedLock => write!(f, "the lock that hold the file is unecpetitly poisoned"),
            CPackError::FourFirstByteNotZero(value) => write!(f, "the four first bytes of the file should be zero, but they are {:?}.", value),
            CPackError::EndOfFileOutOfScope(file_id, end_of_out_file, end_of_source_file) => write!(f, "The file (id: {}) end after the source file end (source file end: {}, output file end in the source file: {})", file_id, end_of_source_file, end_of_out_file),
            CPackError::EndOfHeaderNotZero(start_end_of_header, value) => write!(f, "the end of the header should be 8 zero bytes, but found {:?} (end of the header start at {})", value, start_end_of_header),
            CPackError::PartitionCreationError(_) => write!(f, "unable to create a sub file partition"),
        }
    }
}

impl From<io::Error> for CPackError {
    fn from(err: io::Error) -> CPackError {
        Self::IOError(err)
    }
}

fn cpack_read_u32<F: Read>(file: &mut F) -> Result<u32, CPackError>{
    let mut buffer = [0; 4];
    file.read_exact(&mut buffer)?;
    Ok(u32::from_le_bytes(buffer))
}

#[derive(Debug)]
struct FileIndex {
    file_offset: u32,
    file_lenght: u32,
}

#[derive(Debug)]
/// A structure that represent a cpack file, used in pokemon mystery dungeon games
///
/// Those cpack file are archive that may contain multiple file, each file being identified by an id representing it's order of position in the file.
pub struct CPack<F: Read + Seek> {
    offset_table: Vec<FileIndex>,
    file: Arc<Mutex<F>>,
}

impl<F: Read + Seek> CPack<F> {
    /// Create a CPack struct from a cpack file
    pub fn new_from_file(file: F) -> Result<CPack<F>, CPackError> {
        let mut result = CPack{
            offset_table: Vec::new(),
            file: Arc::new(Mutex::new(file)),
        };
        result.parse()?;
        Ok(result)
    }

    fn parse(&mut self) -> Result<(), CPackError> {
        let mut file = self.file.lock().map_err(|_| CPackError::PoisonedLock)?;

        let file_len = file.seek(SeekFrom::End(0))? as u32;

        file.seek(SeekFrom::Start(0))?;
        let mut first_four_bytes = [1; 4];
        file.read_exact(&mut first_four_bytes)?;
        if first_four_bytes != [0,0,0,0] {
            return Err(CPackError::FourFirstByteNotZero(first_four_bytes));
        };

        let number_of_file = cpack_read_u32(&mut *file)?;

        for file_id in 0..number_of_file {
            let file_offset = cpack_read_u32(&mut *file)?;
            let file_lenght = cpack_read_u32(&mut *file)?;
            if file_offset + file_lenght > file_len {
                return Err(CPackError::EndOfFileOutOfScope(file_id, file_offset + file_lenght, file_len));
            }
            self.offset_table.push(FileIndex {
                file_offset, file_lenght,
            });
        }

        let mut buffer = [1; 8];
        file.read_exact(&mut buffer)?;
        if buffer != [0,0,0,0,0,0,0,0] {
            return Err(CPackError::EndOfHeaderNotZero(file.seek(SeekFrom::Current(0))?, buffer));
        }
        Ok(())
    }

    /// Return the number of file in the cpack archive
    pub fn len(&self) -> usize {
        self.offset_table.len()
    }

    /// Return true if the cpack archive is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// get the file by an id, and return it as PartitionMutex. panic if it doesn't exist
    pub fn get_file(&self, id: usize) -> Result<PartitionMutex<F>, CPackError> {
        let file_data = &self.offset_table[id];
        PartitionMutex::new(
            self.file.clone(),
            file_data.file_offset as u64,
            file_data.file_lenght as u64,
        ).map_err(CPackError::PartitionCreationError)
    }
}

// From the old implementation

/*
#[derive(Debug, Default)]
/// A structure that allow to create a CPack file
pub struct CPackCreator {
    files: Vec<Box<dyn Read + Debug>>,
}

impl CPackCreator {
    /// add a file to the cpack
    pub fn push(&mut self, file: Box<dyn Read>) {
        self.files.push(file);
    }

    /*/// transform the actual content of the [CPackCreator] to a cpack file
    pub fn write(&self) -> Result<Bytes, CPackError> {
        let mut file = Bytes::new();
        file.write_u32_le(0)?;
        file.write_u32_le(self.files.len() as u32)?;
        // file info. Need to be rewritten
        let mut nb = 0;
        for _ in 0..self.files.len() {
            file.write_u32_le(0)?;
            file.write_u32_le(0)?;
            nb += 8;
        };

        // seem to be a padding to 32 bytes
        while nb%32 != 0 {
            file.write_u8_le(0)?;
            nb += 1;
        };

        // another padding to 64 bytes (maybe 128)
        while file.tell()%64 != 0 {
            file.write_u8_le(0xFF)?;
        };


        let mut file_info = vec![];
        for f in &self.files {
            file_info.push(FileIndex {
                file_offset: file.tell() as u32,
                file_lenght: f.len() as u32,
            });
            file.write_bytes(f)?;
            // padding with the len of 16
            let mut nb = f.len();
            while nb%16 != 0 {
                file.write_u8_le(0xFF)?;
                nb += 1;
            }
        };

        file.seek(8);

        for info in file_info {
            file.write_u32_le(info.file_offset)?;
            file.write_u32_le(info.file_lenght)?;
        }

        Ok(file)
    }*/
    //TODO:
}
*/

/*#[test]
fn test_cpack_read() {
    const some_value: [u8; 42] = [0,0,0,0, //0-the magic
        2,0,0,0, //4-the number of element
        32,0,0,0,5,0,0,0, //8-the offset and the lenght of the first element
        37,0,0,0,5,0,0,0, //16-idem for the second element
        0,0,0,0,0,0,0,0, //24-magic
        104,101,108,108,111, //32-b"hello"
        119,111,114,108,100, //37-b"world"
    ];

    let buf = std::io::Cursor::new(some_value);
    let pack = CPack::new_from_file(buf).unwrap();
    assert_eq!(pack.len(), 2);
    let mut string_buffer = String::new();
    pack.get_file(0).unwrap().read_to_string(&mut string_buffer);
    assert_eq!(string_buffer, String::from("hello"));
    pack.get_file(1).unwrap().read_to_string(&mut string_buffer);
    assert_eq!(string_buffer, String::from("world"));
}*/
