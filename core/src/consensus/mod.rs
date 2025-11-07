use crate::storage::block::Block;


pub fn validate_block(block: &Block) -> bool {
    // TODO: PoW/PoS validation, timestamp check, previous hash check, merkle root check, etc.
    !block.transactions.is_empty()
}