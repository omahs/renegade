mod config;
mod gossip;
mod handshake;
mod state;

use crossbeam::channel;
use tokio::sync::mpsc;

use crate::{
    gossip::{
        api::GossipOutbound, 
        GossipServer,
        network_manager::NetworkManager
    },
    handshake::HandshakeManager,
    state::{RelayerState}
};

#[tokio::main]
async fn main() -> Result<(), String> {
    // Parse command line arguments
    let args = config::parse_command_line_args().expect("error parsing command line args");
    println!("Relayer running with\n\t version: {}\n\t port: {}", args.version, args.port);

    // Construct the global state
    let global_state = RelayerState::initialize_global_state(
        args.wallet_ids,
        args.bootstrap_servers
    );

    // Build communication primitives
    let (network_sender, network_receiver) = mpsc::unbounded_channel::<GossipOutbound>();
    let (heartbeat_worker_sender, heartbeat_worker_receiver) = channel::unbounded();

    // Start the network manager
    let network_manager = NetworkManager::new(
        args.port,
        network_receiver,
        heartbeat_worker_sender.clone(),
        global_state.clone()
    ).await.expect("error building network manager");

    // Start the gossip server
    let gossip_server = GossipServer::new(
        network_manager.local_peer_id,
        global_state.clone(),
        heartbeat_worker_sender.clone(),
        heartbeat_worker_receiver,
        network_sender.clone()
    ).await.unwrap();
    
    // Start the handshake manager
    let handshake_manager = HandshakeManager::new(
        global_state.clone(), 
        network_sender.clone()
    );
    
    // Await termination of the submodules
    network_manager.join();
    gossip_server.join();
    handshake_manager.join();
    Err("Relayer terminated...".to_string())
}
