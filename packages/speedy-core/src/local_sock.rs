pub use interprocess::local_socket::tokio::{Listener, Stream};
pub use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Name, ToNsName};
pub use interprocess::local_socket::traits::tokio::{Listener as ListenerTrait, Stream as StreamTrait};

pub fn default_name() -> std::io::Result<Name<'static>> {
    "speedy-daemon".to_ns_name::<GenericNamespaced>()
}
