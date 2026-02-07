pub mod channel_type;
pub mod events;
pub mod queue;

// ChannelType is available for future use - gradual migration from strings
#[allow(unused_imports)]
pub use channel_type::ChannelType;
pub use events::{InboundMessage, OutboundMessage};
pub use queue::MessageBus;
