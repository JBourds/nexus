use super::*;

/// Struct containing routes for every channel.
#[derive(Debug)]
pub struct RoutingTable {
    pub entries: Vec<ChannelRoutes>,
}

/// Struct containing route information for a single channel.
/// A single channel could have potentially many subscribers, each of which are
/// tied to a node class. A given node class can have multiple concrete
/// instances, hence why this is a collection. For each node class (`NodeHandle`)
/// the vector of routes will be a route to every concrete instance of it.
#[derive(Debug)]
pub struct ChannelRoutes {
    /// Mapping of node handles for a node class (`NodeHandle`) to a vector of
    /// routes, where each route is route information to a specific instance of
    /// a node from that class.
    pub nodes: HashMap<NodeHandle, Vec<Route>>,
}

/// Single route from a source node to one destination handle on a channel.
/// Distance is computed dynamically from live node positions at queue/deliver
/// time so that mobile nodes are handled correctly.
#[derive(Clone, Debug)]
pub(crate) struct Route {
    /// Index pointer into the `handles` array for the destination endpoint.
    pub handle_ptr: usize,
}

impl RoutingTable {
    pub(super) fn new(channels: &ResolvedChannels) -> Self {
        let entries = (0..channels.channels.len())
            .map(|index| ChannelRoutes::new(channels, index))
            .collect::<Vec<_>>();
        Self { entries }
    }
}

impl ChannelRoutes {
    fn new(channels: &ResolvedChannels, index: usize) -> Self {
        // For every channel, map every publishing node to the set of
        // precomputed routes it has with every receiving node
        let publishers = &channels.channels[index].publishers;
        let nodes = publishers
            .iter()
            .map(|src_node| (*src_node, Route::outgoing(channels, index, *src_node)))
            .collect::<HashMap<_, _>>();
        Self { nodes }
    }
}

impl Route {
    /// Determine all destination handles reachable by `src_node` on `src_ch`.
    /// Distance is intentionally not stored here; it is computed from live node
    /// positions at queue/deliver time to support mobile nodes.
    fn outgoing(channels: &ResolvedChannels, src_ch: usize, src_node: usize) -> Vec<Self> {
        let ch = &channels.channels[src_ch];
        channels
            .handles
            .iter()
            .enumerate()
            .filter_map(|(handle_ptr, (_, dst_node, dst_ch))| {
                if src_ch == *dst_ch
                    && (ch.subscribers.contains(dst_node)
                        || src_node == *dst_node && ch.r#type.delivers_to_self())
                {
                    Some(Route { handle_ptr })
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    }
}
