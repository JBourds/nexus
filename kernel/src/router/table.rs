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

/// Single route containing computed information about an endpoint relative to
/// a single node and distance from it. Uses `handle_ptr` to lookup the channel
/// information and compute appropriate delays/bit errors/dropped packets at
/// runtime (nodes could move).
#[derive(Clone, Debug)]
pub(crate) struct Route {
    /// Index pointer into the `handles` array for the specific channel the
    /// route is connected to.
    pub handle_ptr: usize,
    /// Euclidean distance between the nodes.
    pub distance: f64,
    /// Units for `distance` field
    pub unit: DistanceUnit,
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
            .map(|src_node| (index, Route::outgoing(channels, index, *src_node)))
            .collect::<HashMap<_, _>>();
        Self { nodes }
    }
}

impl Route {
    /// Function to determine the set of channels which should be reached by
    /// `src_node` transmitting information to `src_ch` and compute route
    /// information. Called once at setup.
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
                    let src = &channels.nodes[src_node];
                    let dst = &channels.nodes[*dst_node];
                    let (distance, unit) = Position::distance(&src.position, &dst.position);
                    Some(Route {
                        handle_ptr,
                        distance,
                        unit,
                    })
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    }
}
