# async/await-friendly circular buffer

This library provides a [circular
buffer](https://en.wikipedia.org/wiki/Circular_buffer) that implements
`AsyncRead` and `AsyncWrite`.  It's meant to be useful to embedded applications
which want to use async/await but for which the full might of tokio would be
overkill.

It's currently in a "`no_std`-ish" state, where I've put some thought into
making it work for `no_std` + `alloc` but haven't tested it much in that
context.  It makes two allocations at buffer creation time (one for the
context, one for the storage), and no allocations in normal usage.

Ultimately the goal would be to eliminate the `alloc` dependency, perhaps by
leveraging [heapless](https://docs.rs/heapless/0.5.1/heapless/) or
[arrayvec](https://docs.rs/arrayvec/0.5.1/arrayvec/) for the storage and
learning when stack-object pinning is safe.
