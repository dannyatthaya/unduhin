//! The one place the bridge's two transports differ.
//!
//! Windows uses a named pipe, everything else a Unix domain socket. Both
//! yield a stream that splits into independent read and write halves, so
//! [`super`] can stay entirely platform-neutral.
//!
//! The accept semantics genuinely differ, which is why this is a struct
//! rather than a type alias. A `NamedPipeServer` *is* the pre-created
//! instance and `connect()` consumes its readiness, so a fresh instance
//! has to be minted per connection. A `UnixListener` is persistent and
//! `accept()` hands back a new stream. [`Listener`] hides that.

use std::io;

/// The server end of one accepted connection.
#[cfg(windows)]
pub(super) type ServerStream = tokio::net::windows::named_pipe::NamedPipeServer;
#[cfg(unix)]
pub(super) type ServerStream = tokio::net::UnixStream;

#[cfg(unix)]
pub(super) use unix_impl::Listener;
#[cfg(windows)]
pub(super) use windows_impl::Listener;

#[cfg(windows)]
mod windows_impl {
    use super::{io, ServerStream};
    use std::time::Duration;
    use tokio::net::windows::named_pipe::ServerOptions;

    pub(in crate::pipe) struct Listener {
        name: String,
        security: crate::pipe::pipe_security::PipeSecurity,
        /// The instance currently waiting for a client. Named pipes hand
        /// out readiness once, so this is swapped for a fresh instance
        /// after every accept.
        pending: ServerStream,
    }

    impl Listener {
        pub(in crate::pipe) async fn bind(name: &str) -> io::Result<Self> {
            // Build the restrictive descriptor once; every instance is
            // created with it. Fail closed — we never fall back to a
            // permissive pipe.
            let security = crate::pipe::pipe_security::PipeSecurity::current_user_only()?;
            let pending = Self::make(name, &security, true)?;
            Ok(Self {
                name: name.to_string(),
                security,
                pending,
            })
        }

        fn make(
            name: &str,
            security: &crate::pipe::pipe_security::PipeSecurity,
            first: bool,
        ) -> io::Result<ServerStream> {
            let mut opts = ServerOptions::new();
            if first {
                // The first instance owns the well-known name; later ones
                // must not claim it or they collide.
                opts.first_pipe_instance(true);
            }
            // SAFETY: `security` outlives this call, so the attributes
            // pointer stays valid for the duration of the create.
            unsafe { opts.create_with_security_attributes_raw(name, security.as_attrs_ptr()) }
        }

        /// Recreate an instance, retrying transient failures with capped
        /// backoff. A single listener error must never permanently
        /// disable the bridge, so this never gives up.
        async fn recreate(&self) -> ServerStream {
            let mut backoff = Duration::from_millis(100);
            loop {
                match Self::make(&self.name, &self.security, false) {
                    Ok(server) => return server,
                    Err(e) => {
                        tracing::warn!(error = %e, backoff_ms = backoff.as_millis(),
                            "failed to (re)create pipe instance; retrying");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(5));
                    }
                }
            }
        }

        pub(in crate::pipe) async fn accept(&mut self) -> io::Result<ServerStream> {
            loop {
                // On a connect error the handle is unusable, so rebuild it
                // rather than tearing down the whole server.
                if let Err(e) = self.pending.connect().await {
                    tracing::warn!(error = %e, "pipe connect failed; recreating listener");
                    self.pending = self.recreate().await;
                    continue;
                }
                // Hand the connected instance out and immediately stand up
                // a fresh one for the next client.
                let fresh = self.recreate().await;
                return Ok(std::mem::replace(&mut self.pending, fresh));
            }
        }
    }
}

#[cfg(unix)]
mod unix_impl {
    use super::{io, ServerStream};
    use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    pub(in crate::pipe) struct Listener {
        inner: tokio::net::UnixListener,
        path: PathBuf,
    }

    impl Listener {
        pub(in crate::pipe) async fn bind(path: &str) -> io::Result<Self> {
            unduhin_core::wire::transport::check_endpoint_len(path)?;
            let path = PathBuf::from(path);

            // Create the parent directory private if it is ours to create.
            //
            // Deliberately only on creation. Tightening an existing
            // directory would be a footgun: `UNDUHIN_PIPE_NAME` can point
            // anywhere, and chmod-ing a shared directory such as `/tmp`
            // down to 0700 would break the system for every other process.
            // A directory that already exists keeps its permissions, and
            // the socket's own 0600 plus the peer-uid check in `accept`
            // carry the protection.
            if let Some(dir) = path.parent() {
                if !dir.exists() {
                    std::fs::DirBuilder::new()
                        .recursive(true)
                        .mode(0o700)
                        .create(dir)?;
                }
            }

            Self::clear_stale(&path)?;
            let inner = tokio::net::UnixListener::bind(&path)?;
            // Belt and braces on top of the directory mode: macOS and the
            // BSDs do enforce socket file permissions on connect.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;

            Ok(Self { inner, path })
        }

        /// Remove a leftover socket file, but only once we are sure it is
        /// ours and dead.
        ///
        /// This is the common case, not the exceptional one: `lib.rs`
        /// ends in `std::process::exit(0)`, which skips every destructor,
        /// so a clean quit still leaves the file behind and `bind` would
        /// fail with `EADDRINUSE`.
        ///
        /// Liveness is only decidable by trying to connect. A refused
        /// connection means nothing is listening; a successful one means a
        /// second instance is running and we must not steal its socket.
        fn clear_stale(path: &Path) -> io::Result<()> {
            // `symlink_metadata`, never `metadata`: following a symlink
            // here would let us unlink someone else's file.
            let meta = match std::fs::symlink_metadata(path) {
                Ok(m) => m,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e),
            };

            if !meta.file_type().is_socket() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a socket", path.display()),
                ));
            }
            if meta.uid() != unsafe { libc::geteuid() } {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{} is owned by another user", path.display()),
                ));
            }

            match std::os::unix::net::UnixStream::connect(path) {
                Ok(_) => Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!(
                        "another Unduhin instance is already listening on {}",
                        path.display()
                    ),
                )),
                Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
                    // Nobody home: the file is a leftover.
                    std::fs::remove_file(path)
                }
                Err(e) => Err(e),
            }
        }

        pub(in crate::pipe) async fn accept(&mut self) -> io::Result<ServerStream> {
            loop {
                let (stream, _addr) = self.inner.accept().await?;

                // Identity check against the connecting process, which is
                // the honest parity with what the Windows DACL intends:
                // an OS-enforced answer that does not depend on the
                // filesystem still being in the state we left it.
                match peer_uid(&stream) {
                    Ok(uid) if uid == unsafe { libc::geteuid() } => return Ok(stream),
                    Ok(uid) => {
                        tracing::warn!(uid, "rejecting bridge connection from another user");
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "could not identify bridge peer; rejecting");
                        continue;
                    }
                }
            }
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            // Covers the ordinary paths. The hard-exit path cannot rely on
            // this and calls `super::shutdown()` instead.
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn peer_uid(stream: &tokio::net::UnixStream) -> io::Result<libc::uid_t> {
        use std::os::fd::AsRawFd;
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        // SAFETY: `stream` owns a live socket fd for the duration of this
        // call, and both out-params are valid initialized locals.
        let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(uid)
    }
}
