use std::convert::TryFrom;
use std::sync::Arc;

use focus_formats::storage::{
    ContentDigest,
    Wants,
};
use focus_formats::storage::get_inline;
use focus_formats::storage::persisted;
use internals::error::AppError;
use internals::storage::rocks::{Storage, Keygen};
use prost::Message;

#[derive(Debug)]
pub struct ObjectStore {
    storage: Arc<Storage>,
    keygen: Keygen,
}


// two record types: header and body
//
// header key is "o:${version}:${OID}:h"
// * "o"      - literal 'o' char - 0x6f
// * $version - u8 version number of the data format: '1'
// * $OID     - the raw bytes of the git object ID (20 bytes for SHA1)
// * "h"      - the literal char 'h' - 0x68
//
// header value "${TYPE_BYTE}${OBJECT_SIZE}"  (fixed size 5 bytes)
//
// * TYPE_BYTE: one of the 4 git data types as its native enum value
//     Commit = 1,
//     Tree = 2,
//     Blob = 3,
//     Tag = 4,
//
// * OBJECT_SIZE: a u32 MSB encoded value for the size of the object
//
// body key is "o:${version}:${OID}:b"
//
// * same meanings as the header key except with the 'b' suffix for 'body'
//

const COMMIT_STR: &str = "commit";
const TREE_STR: &str = "tree";
const BLOB_STR: &str = "blob";
const TAG_STR: &str = "tag";

fn obj_type_to_str(ot: persisted::ObjectType) -> &'static str {
    use persisted::ObjectType::*;
    match ot {
        Commit => COMMIT_STR,
        Tree => TREE_STR,
        Blob => BLOB_STR,
        Tag => TAG_STR,
        None => panic!("BUG: None object type encountered")
    }
}

fn headers_to_git_header_bytes(hdr: persisted::Headers) -> Vec<u8> {
    use persisted::ObjectType;
    let s = format!(
        "{} {}\x00",
        obj_type_to_str(ObjectType::from_i32(hdr.obj_type).unwrap()),
        hdr.size,
    );

    s.into_bytes()
}

fn decode_header(buf: &[u8]) -> Result<persisted::Headers, AppError> {
    persisted::Headers::decode(buf)
        .map_err(|err| AppError::DecodeError(err))
}

const GET_INLINE_NOT_FOUND: get_inline::Response = get_inline::Response {
    found: false,
    size: 0,
    header: vec![],
    body: vec![],
};

impl ObjectStore {
    pub fn new(storage: Storage, keygen: Keygen) -> ObjectStore {
        ObjectStore {
            storage: Arc::new(storage),
            keygen,
        }
    }

    fn get_inline_header_result(&self, key: &[u8]) -> Result<Option<get_inline::Response>, AppError> {
        match self.storage.get_bytes(key) {
            Ok(Some(bytes)) => {
                let hdr = decode_header(&bytes)?;
                let rep = get_inline::Response {
                    found: true,
                    size: u32::try_from(hdr.size)?,
                    header: headers_to_git_header_bytes(hdr),
                    body: vec![]
                };
                Ok(Some(rep))
            },
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn get_inline_opt(
        &self,
        oid: &ContentDigest,
        wants: Wants
    ) -> Result<Option<get_inline::Response>, AppError> {
        let key = self.keygen.key_for(&oid.value);

        let opt = match self.get_inline_header_result(&key.for_header())? {
            Some(mut rep) => {
                match wants {
                    Wants::Header => Some(rep), // this is a little silly but it lets us do the rest
                    Wants::None => {
                        rep.header = vec![];
                        Some(rep)
                    },
                    Wants::Body => {
                        match self.storage.get_bytes(&key.for_body())? {
                            Some(body) => {
                                rep.header = vec![];
                                rep.body = body.to_vec();
                                Some(rep)
                            },
                            None => None,
                        }
                    },
                    Wants::Both => {
                        match self.storage.get_bytes(&key.for_body())? {
                            Some(body) => {
                                rep.body = body.to_vec();
                                Some(rep)
                            }
                            None => None
                        }
                    },
                }
            },
            None => None,
        };

        Ok(opt)
    }

    pub fn get_inline(
        &self,
        oid: &ContentDigest,
        wants: Wants
    ) -> Result<get_inline::Response, AppError> {
        self.get_inline_opt(oid, wants).map(|opt| {
            match opt {
                Some(rep) => rep,
                None => GET_INLINE_NOT_FOUND,
            }
        })
    }
}

#[cfg(tests)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn mk_storage() -> Result<ObjectStore, Error> {
        let d = tempdir()?;
        Storage::new(d)




    }
}
