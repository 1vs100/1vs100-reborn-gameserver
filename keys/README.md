# keys/

`grid_lan_private.pem` is the private half of the RSA key the Grid and livehost replies are
signed with. It is committed and compiled into the server (`server.rs`) **on purpose: it is not a
secret.**

- **Where it comes from:** the console-side patch (`1vs100-x360-sysdll`,
  `sysdll/patches/BackendRedirect.cpp`) replaces the two Microsoft Grid RSA public keys in the
  v2.0.1317.0 title with this key's public half (`kGridLanModulus`, exponent 65537). A 1317
  console then accepts only replies signed with this key. v2.0.1149.0 does not sign its Grid
  requests and is not patched.
- **Do not rotate it.** A new key breaks every patched console until the patch carries the new
  modulus. `tests/grid_key.rs` fails if the key no longer matches the patched modulus.
- **Anyone can sign as this server with it.** That is fine for a LAN revival server; it is not a
  trust anchor for anything else.
