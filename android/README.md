# netonet on Android

On Android the kernel TUN device is owned by the system. Your app requests one
through `VpnService`, which returns a `ParcelFileDescriptor` already configured
with the overlay address, routes and MTU. `netonet-mobile` consumes that file
descriptor and runs the standard netonet engine on it.

## Native library

`crates/netonet-mobile` builds a `cdylib` exposing two JNI functions for the
class `com.netonet.NetonetVpn`:

```kotlin
external fun nativeStart(fd: Int, configToml: String): Boolean
external fun nativeStop()
```

`configToml` uses the same schema as the desktop config, except the
`[interface]` section is ignored (the VpnService configures the interface).

### Cross-compiling

Use [`cargo-ndk`](https://github.com/bbqsrc/cargo-ndk) with the Android NDK:

```sh
cargo install cargo-ndk
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
    -o app/src/main/jniLibs \
    build --release -p netonet-mobile
```

The resulting `libnetonet_mobile.so` files go under `jniLibs/<abi>/`.

## VpnService skeleton (Kotlin)

```kotlin
package com.netonet

import android.net.VpnService
import android.content.Intent
import android.os.ParcelFileDescriptor

class NetonetVpn : VpnService() {
    private var tun: ParcelFileDescriptor? = null

    companion object {
        init { System.loadLibrary("netonet_mobile") }
        @JvmStatic external fun nativeStart(fd: Int, configToml: String): Boolean
        @JvmStatic external fun nativeStop()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val configToml = intent?.getStringExtra("config") ?: return START_NOT_STICKY

        val pfd = Builder()
            .setSession("netonet")
            .setMtu(1380)
            .addAddress("10.7.0.3", 24)       // this device's overlay IP
            .addRoute("10.7.0.0", 24)         // route the overlay subnet into the tunnel
            .establish() ?: return START_NOT_STICKY

        tun = pfd
        // detachFd() transfers ownership of the fd to native code.
        nativeStart(pfd.detachFd(), configToml)
        return START_STICKY
    }

    override fun onDestroy() {
        nativeStop()
        tun?.close()
        tun = null
        super.onDestroy()
    }
}
```

Notes:

- `detachFd()` hands the descriptor to Rust; do not close the `ParcelFileDescriptor`
  copy afterwards.
- Add the `android.permission.INTERNET` permission and register the service with
  the `android.permission.BIND_VPN_SERVICE` permission in your manifest, and call
  `VpnService.prepare()` from your activity before starting it.
- Logs are emitted to logcat under the tag `netonet`.
