use super::*;

#[derive(Debug)]
pub struct RoutingTable {
    pub entries: Vec<ChannelRoutes>,
}

#[derive(Debug)]
pub struct ChannelRoutes {
    pub nodes: HashMap<NodeHandle, Vec<Route>>,
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

#[derive(Clone, Debug)]
pub(crate) struct Route {
    pub handle_ptr: usize,
    pub distance: f64,
    pub unit: DistanceUnit,
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
