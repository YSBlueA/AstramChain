use libp2p::{Swarm, PeerId, identity};


pub fn init_network() -> (PeerId, Swarm<()>) {
let local_key = identity::Keypair::generate_ed25519();
let peer_id = PeerId::from(local_key.public());
// TODO: Swarm and peer discovery settings
(peer_id, todo!())
}