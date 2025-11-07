use rocksdb::{DB, Options, WriteBatch};
use anyhow::Result;
use std::path::Path;

/// key rule (string keys)
/*
 Keys:
  h:<block_hash> -> serialized header (bincode)
  i:<height> -> block_hash (utf8)
  t:<txid> -> serialized tx (bincode)
  u:<txid>:<vout> -> serialized UTXO (bincode)
  tip -> block_hash
*/

pub fn open_db(path: &str) -> Result<DB, anyhow::Error> {
    let mut opts = Options::default();
    opts.create_if_missing(true);
    let p = Path::new(path);
    let db = DB::open(&opts, p)?;
    Ok(db)
}

pub fn put_batch(db: &DB, batch: WriteBatch) -> Result<(), anyhow::Error> {
    db.write(batch)?;
    Ok(())
}
