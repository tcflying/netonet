//! Android JNI bindings for netonet.
//!
//! On Android the TUN device is created by the system `VpnService`: the app
//! calls `VpnService.Builder.establish()` which returns a file descriptor that
//! is already configured with the overlay IP, routes and MTU. We hand that fd
//! to [`tun_rs::AsyncDevice::from_fd`] and run the same engine as on desktop.
//!
//! Expected Kotlin/Java side (package `com.netonet`, class `NetonetVpn`):
//!
//! ```text
//! external fun nativeStart(fd: Int, configToml: String): Boolean
//! external fun nativeStop()
//! ```
//!
//! The config TOML uses the same schema as the desktop node, but the
//! `[interface]` section is ignored (the interface is configured by the
//! VpnService on the Kotlin side).

use std::os::fd::RawFd;
use std::sync::{Arc, Mutex, OnceLock};

use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jint, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use netonet_core::{build_endpoint, run, Config};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

struct Running {
    runtime: Runtime,
    task: JoinHandle<()>,
}

static STATE: OnceLock<Mutex<Option<Running>>> = OnceLock::new();

fn state() -> &'static Mutex<Option<Running>> {
    STATE.get_or_init(|| Mutex::new(None))
}

fn init_logging() {
    #[cfg(target_os = "android")]
    {
        // Route `log` records (and tracing-via-log) to logcat under tag "netonet".
        android_logger::init_once(
            android_logger::Config::default()
                .with_tag("netonet")
                .with_max_level(log::LevelFilter::Info),
        );
    }
}

/// Starts the engine. Returns `true` on success.
///
/// # Safety
/// `fd` must be a valid, owned TUN file descriptor produced by
/// `VpnService.Builder.establish()`. Ownership is transferred to native code.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_netonet_NetonetVpn_nativeStart(
    mut env: JNIEnv,
    _class: JClass,
    fd: jint,
    config_toml: JString,
) -> jboolean {
    init_logging();

    let config_str: String = match env.get_string(&config_toml) {
        Ok(s) => s.into(),
        Err(err) => {
            tracing::error!(error = %err, "failed to read config string");
            return JNI_FALSE;
        }
    };

    match start(fd as RawFd, &config_str) {
        Ok(()) => JNI_TRUE,
        Err(err) => {
            tracing::error!(error = %err, "failed to start netonet");
            JNI_FALSE
        }
    }
}

/// Stops the engine and releases resources.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_netonet_NetonetVpn_nativeStop(
    _env: JNIEnv,
    _class: JClass,
) {
    if let Some(running) = state().lock().unwrap().take() {
        running.task.abort();
        // Shut the runtime down without blocking the calling (UI) thread forever.
        running.runtime.shutdown_background();
    }
}

fn start(fd: RawFd, config_str: &str) -> anyhow::Result<()> {
    let mut guard = state().lock().unwrap();
    if guard.is_some() {
        anyhow::bail!("netonet already running");
    }

    let config: Config = toml::from_str(config_str)?;
    let secret_key = config.secret_key()?;
    let relay = config.relay()?;
    let routes = config.routes()?;

    // SAFETY: the fd is an owned TUN descriptor from VpnService (see method docs).
    let device = unsafe { tun_rs::AsyncDevice::from_fd(fd)? };
    let device = Arc::new(device);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let task = runtime.spawn(async move {
        let endpoint = match build_endpoint(secret_key, relay.clone()).await {
            Ok(ep) => ep,
            Err(err) => {
                tracing::error!(error = %err, "failed to build endpoint");
                return;
            }
        };
        endpoint.online().await;
        if let Err(err) = run(endpoint, device, routes, relay).await {
            tracing::error!(error = %err, "engine stopped");
        }
    });

    *guard = Some(Running { runtime, task });
    Ok(())
}
