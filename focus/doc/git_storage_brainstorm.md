# Git Storage


## Blobs

Databases are not especially good at storing blobs. We're in the territory of new development.

- RocksDB BlobDB: https://rocksdb.org/blog/2021/05/26/integrated-blob-db.html 
- PostgreSQL TOAST: https://www.postgresql.org/docs/current/storage-toast.html

It occured to me that for RocksDB BlobDB, it might be worth trying to see if we can use pinned storage

## Concurrent Access

In Rocks, it is possible to open from multiple processes for read, but not for write. We may be able to exploit this to use pinned memory in the client code rather than doing RPC, etc. Problems arise for writing objects, in which case the process would need to open as the primary. Understanding if you can open as a secondary without a primary is unclear to me from what I've read, but sounds possible.

https://github.com/facebook/rocksdb/wiki/Secondary-instance


