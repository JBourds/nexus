#align(center, [
  = Testnet Pitch
  \
  Jordan Bourdeau
])


== Abstract

Testing/deploying networking code with physical devices is constrained by the
time it takes to upload code to devices, physically position them, and any other
time-intensive battles due to quirks in many end systems.
In order to quickly test network topologies and protocols, a robust
simulation program is required. Many existing simulators do not quite fit
the niche of the IoT network protocol developer. They tend to be limited
in the types of networks they can simulate (e.g., Just Ethernet/IP as with Mininet)
or lack the expressivity to properly test network protocols.
The key difference in this
simulator would be the ability to create arbitrary types of network links
and test the actual protocol code which would be deployed. An additional
benefit of this structure is the ease in which deterministic simulation testing
(fuzzing) can be setup for every subcomponent of a network.

== Configuration

> I've implemented a sample configuration and most of the parsing/validation for this already.

A network can be configured using a TOML file, and is representable as a series
of *nodes*, *protocols*, and *links*.

*Node:* A class of host within a network deployed in either physical locations
((x,y) coordinates within the simulation) or abstractly (e.g., Satellites
which we may not want to represent as being physically present in a grid,
but may still want to have direct links to from nodes).
Nodes are characterized by the set of protocols they provide for communicating
with each other. Nodes can also have internal links, which act as links
within that node's namespace to communicate between protocols in the same node
(comparable to how a program with multiple modules would call upon different
services).

*Protocol:* An abstract unit of execution within a node that directly maps to
a process on the computer hosting the simulation. As part of the input, it takes
a root directory and a runner script to start the process.
Protocols are characterized by the node they belong to, inbound communication
channels (*links*), and its direct/indirect outbound links.

*Link:* An abstraction of one or more communication mediums in a network. Can be *direct* (corresponding to a guided medium) or *indirect*
(corresponding to an unguided medium). Allows for the simulation of:

- Network delays (queue, processing, processing, etc.)
- Transmission errors (bit flips, packet loss, etc.)
- Transmission rates
- Intermediary connections
  - Allows specifying the next link type, and the number of intermediary links
  of that type.


= Simulation

> Have not started on the actual simulation, so the details are fuzzy.

*TLDR:*

The simulation will transmit data written to the outbound links of a
protocol/process (likely corresponding to files/pipes acting like a mock for
a socket) according to the rules in the configuration. For instance,
the following configuration would be roughly the same as a normal Unix pipe,
except any date written by a protocol in node A to node B over this link
would be delivered to all protocols in node B which accept that link.

Node A ---> Node B
- Connected over an ideal link, no delays, bit errors, or packet loss.

If node A has a single protocol and node B has 3, the information flow can
be visualized as:

```
Protocol A.1 ---------> Protocol B.1
                 |
  (Ideal Link)   -----> Protocol B.2
                 |
                 -----> Protocol B.3
```

Each of the processes running these protocols would then be responsible for
taking the appropriate action for the protocol.
