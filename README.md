# mpus backend

The Rust service behind the mpus cat app. It owns accounts, cat records, photo storage,
cat re-identification, email, push, moderation, and merges. The Android client is a
separate repo.

<div align="center">

[![Rust](https://img.shields.io/badge/Rust-2024-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![axum](https://img.shields.io/badge/axum-0.8-000000?style=for-the-badge)](https://github.com/tokio-rs/axum)
[![tokio](https://img.shields.io/badge/tokio-1-000000?style=for-the-badge&logo=tokio&logoColor=white)](https://tokio.rs/)
[![sqlx](https://img.shields.io/badge/sqlx-0.8-31648C?style=for-the-badge)](https://github.com/launchbadge/sqlx)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-pgvector-4169E1?style=for-the-badge&logo=postgresql&logoColor=white)](https://github.com/pgvector/pgvector)
[![ONNX Runtime](https://img.shields.io/badge/ONNX%20Runtime-2.0--rc.13-005CED?style=for-the-badge&logo=onnx&logoColor=white)](https://onnxruntime.ai/)
[![argon2](https://img.shields.io/badge/argon2-0.5-2A4D69?style=for-the-badge)](https://github.com/RustCrypto/password-hashes)
[![Resend](https://img.shields.io/badge/Resend-email-000000?style=for-the-badge&logo=resend&logoColor=white)](https://resend.com/)
[![Firebase](https://img.shields.io/badge/Firebase%20Messaging-push-FFCA28?style=for-the-badge&logo=firebase&logoColor=black)](https://firebase.google.com/)

</div>

---

## Table of Contents

- [Stack](#stack)
- [Layout](#layout)
- [Build and run](#build-and-run)
- [Configuration](#configuration)
- [Request and response format](#request-and-response-format)
- [Auth and authorization](#auth-and-authorization)
- [Endpoints](#endpoints)
- [Cat re-identification](#cat-re-identification)
- [Photo pipeline](#photo-pipeline)
- [Email](#email)
- [Push notifications](#push-notifications)
- [Rate limiting](#rate-limiting)
- [Database and migrations](#database-and-migrations)
- [Tests](#tests)
- [Deployment](#deployment)
- [License](#license)

---

## Stack

| Component | Version |
|---|---|
| Rust | edition 2024 |
| axum | 0.8 (multipart) |
| tokio | 1 |
| sqlx | 0.8 (postgres, rustls, migrate, macros) |
| PostgreSQL | with pgvector |
| argon2 | 0.5 (argon2id) |
| image + webp | decode, crop, re-encode |
| ort | 2.0.0-rc.13 (ONNX Runtime) |
| reqwest | 0.12 (Resend email API) |
| jsonwebtoken | 9 (FCM OAuth2, RS256) |
| tower-http | 0.6 (trace, fs) |

## Layout

```text
src/
├─ main.rs              entry, tracing, migrations, listener
├─ router.rs            the axum router
├─ state.rs             AppState (db pool, config, http client, embedder, rate limiter)
├─ config.rs            env parsing
├─ error.rs             AppError and its IntoResponse
├─ response.rs          the {success, data} / {success, message} envelope
├─ validation.rs        email, username, password, nickname, cat name
├─ paging.rs            cursor encode/decode and limit clamping
├─ ratelimit.rs         in-memory per-key sliding window
├─ auth/
│  ├─ password.rs       argon2id hash and verify, with a dummy-hash timing equalizer
│  ├─ session.rs        opaque session tokens, sha256 at rest
│  └─ tokens.rs         random opaque tokens and 6-digit email codes
├─ middleware/
│  └─ auth.rs           AuthUser and VerifiedUser extractors
├─ handlers/            auth, email, profile, sightings, cats, merge, moderation, notifications
├─ models/              user, cat
└─ services/            cat, sighting, embedding, image, mail, push, notification,
                        merge, moderation, user
migrations/             0001..0006, applied on startup
models/                 megadescriptor-t-224-int8.onnx
tools/                  export_onnx.py
deploy/                 mpus.service, nginx-mpus.conf
.github/workflows/      deploy.yml
```

## Build and run

Needs Rust, PostgreSQL 16 with the `pgvector` extension, and the ONNX model in `models/`.

```shell
cp .env.example .env        # fill it in, never commit it
cargo run                   # migrations apply on startup
cargo build --release
```

## Configuration

All config is environment variables, parsed in `src/config.rs`. See `.env.example` for the
full set.

| Group | Variables |
|---|---|
| Server | `HOST`, `PORT`, `PUBLIC_BASE_URL` |
| Database | `DATABASE_URL`, `DB_MAX_CONNECTIONS` |
| Email | `RESEND_API_KEY`, `MAIL_FROM` |
| Sessions | `SESSION_TTL_DAYS` |
| Avatars | `UPLOAD_DIR`, `AVATAR_MAX_BYTES`, `AVATAR_MAX_DIMENSION`, `AVATAR_WEBP_QUALITY` |
| Password hashing | `ARGON2_MEMORY_KIB`, `ARGON2_TIME_COST`, `ARGON2_PARALLELISM` |
| Token lifetimes | `VERIFY_TOKEN_TTL_SECS`, `RESET_TOKEN_TTL_SECS` |
| Rate limits | `RATE_*_MAX`, `RATE_*_WINDOW_SECS` (signup, login, resend, forgot, sighting) |
| Embedding | `EMBED_MODEL_PATH`, `EMBED_THREADS`, `EMBED_ARENA`, `EMBED_TOP_K` |
| Sightings | `SIGHTING_MAX_BYTES`, `SIGHTING_MAX_DIMENSION`, `SIGHTING_WEBP_QUALITY`, `SIGHTING_THUMB_MAX_DIM` |
| Push | `FCM_SERVICE_ACCOUNT_JSON` (optional; push is off when unset) |

Secrets live only in `.env` (mode 600). Never commit them, never log them.

## Request and response format

Success:

```json
{ "success": true, "data": { } }
{ "success": true, "message": "..." }
```

Failure:

```json
{ "success": false, "message": "..." }
```

Caveat: axum's own extractor rejections (400, 404, 405, 413, 422) come back as plain text,
not the JSON envelope. The Android client handles both shapes.

## Auth and authorization

- Signup creates the user, sends a verification code by email, and returns `{token, user}`
  right away. The client never bounces back to the form.
- Login does not block unverified accounts. It returns the session plus `user.email_verified`
  so the client can hold the account on a verify screen.
- Verification codes are 6 digits (`tokens::random_code`), stored sha256-hashed like every
  other token. `POST /auth/verify-email` returns the updated user.
- Sessions are opaque 64-hex tokens. Only the sha256 hex is stored, in
  `sessions.token_hash`.
- The token goes out as `Authorization: Bearer <token>`.

Two extractors enforce access:

- `AuthUser` resolves the bearer token. Reads use it. An unverified account may browse.
- `VerifiedUser` wraps `AuthUser` and rejects an unconfirmed account with 403. Every write
  handler takes `VerifiedUser`, so the real gate is server-side; the client verify screen is
  only a convenience.

Passwords are argon2id, with a dummy-hash comparison on unknown accounts so login timing
does not leak whether an email exists.

## Endpoints

```text
GET    /health
POST   /auth/signup
POST   /auth/login
POST   /auth/logout
POST   /auth/verify-email
POST   /auth/resend-verification
POST   /auth/forgot-password
POST   /auth/reset-password
POST   /devices                   {token, platform?}  register this device's FCM token
POST   /devices/delete            {token}             forget it, on sign out
GET    /me
PATCH  /me
POST   /me/avatar                 multipart "avatar"
DELETE /me/avatar
GET    /me/sightings              ?limit=&cursor=
GET    /users/{id}
POST   /sightings                 multipart "photo", lat, lng, taken_at?
POST   /sightings/{id}/resolve    {cat_id? | name?}
GET    /cats?min_lat&min_lng&max_lat&max_lng
GET    /cats/{id}
POST   /cats/{id}/names           {name}
POST   /cats/{id}/reviews         {body, rating 0..10}
POST   /names/{id}/like
POST   /reviews/{id}/like
GET    /avatars/*                 static file server for uploaded photos
```

## Cat re-identification

Photos are embedded with MegaDescriptor-T (Swin-Tiny, 768-d), exported to ONNX with
`tools/export_onnx.py`. Embeddings are stored in Postgres via `pgvector` with an HNSW
cosine index.

`POST /sightings` embeds the photo, inserts the sighting, and returns the top-k most similar
existing cats whose cosine similarity clears `EMBED_TOP_K` / the similarity threshold. The
client decides whether to link or create; nothing is linked automatically.

The embedding reads the original full frame, while the stored photo is center-cropped to
1:1 (see below), so the model still sees the whole cat.

## Photo pipeline

`services/image.rs`:

- `to_webp(bytes, max_dimension, quality)`: decode and re-encode to lossy WebP, dimensions
  capped. Used for avatars.
- `square_webp(bytes, max_dim, quality)`: center-crop to 1:1, then downscale only if past
  `max_dim`. Used for stored sighting photos, so the saved file matches the square preview
  everywhere in the app.
- `thumbnail(bytes, max_dim, quality)`: center-crop to a square and downscale. Used for map
  markers.

Decoding goes through the `image` crate with dimension limits (decompression bomb guard),
not the declared content type. Uploads are written under `UPLOAD_DIR` and served at
`/avatars/*`.

## Email

Resend, via `services/mail.rs`. Verification and reset codes are 6 digits, stored hashed
with a TTL (`VERIFY_TOKEN_TTL_SECS`, `RESET_TOKEN_TTL_SECS`). `send_verification_code` is
shared by signup and resend.

## Push notifications

Firebase Cloud Messaging HTTP v1. The service mints an OAuth2 token (RS256) from
`FCM_SERVICE_ACCOUNT_JSON`. `services/notification.rs` writes the inbox row and fires the
push (spawned, best effort). The push title varies by kind: Name love, New name, New
review, Merge request, Merge update. The payload carries `kind` and `cat_id` for the
client's deep link. Device tokens live in `device_tokens`, keyed by token, and move to the
new user on re-registration.

## Rate limiting

`ratelimit.rs` is an in-memory per-key sliding window. Keys are `user_id` or IP plus an
action label. Limits come from the `RATE_*` config. It is per-process, so a multi-instance
deploy would need shared storage.

## Database and migrations

| Migration | Contents |
|---|---|
| `0001_init.sql` | users, sessions, email_tokens |
| `0002_cats.sql` | `CREATE EXTENSION vector`; cats, sightings (vector(768) + HNSW cosine index), cat_names, name_likes |
| `0003_name_like_uniqueness.sql` | one name-like per user per cat (`UNIQUE (user_id, cat_id)`) |
| `0004_cat_reviews.sql` | cat_reviews (0 to 10, one per user per cat) + review_likes (no cap) |
| `0005_moderation_merge_notifications.sql` | reports (auto-hide at 3 distinct reporters), `cats.merged_into` + merge tables, notifications inbox |
| `0006_device_tokens.sql` | device_tokens for FCM push |

Applied automatically on startup via `sqlx::migrate!`.

## Tests

```shell
cargo test
```

Covers validation, paging, rate limiting, password hashing, token generation, image
encoding and cropping, and the pgvector literal. The embedding test is ignored by default
because it needs the ONNX model present.

## Deployment

The backend runs as a systemd service behind nginx. Templates for both live in `deploy/`
(`mpus.service`, `nginx-mpus.conf`); they are the source of truth for how the box is set
up, and the paths inside them are the ones the CI workflow expects.

### Automatic deploy

`.github/workflows/deploy.yml` ships the binary on every push to `main` (or `master`), and
can also be run by hand from the Actions tab. It builds with `--locked`, copies the binary
over SSH, swaps it in, restarts the service, and checks the health URL.

Everything target-specific is a repository variable, so moving to another server is a
settings change, not a code change. Set these in Settings > Secrets and variables >
Actions:

| Kind | Name | Purpose |
|---|---|---|
| Secret | `SSH_PRIVATE_KEY` | Private key that can log in to the VM |
| Variable | `DEPLOY_HOST` | Server host or IP |
| Variable | `DEPLOY_USER` | SSH user |
| Variable | `DEPLOY_PATH` | App directory on the server, e.g. `/home/sana/mpus` |
| Variable | `DEPLOY_BINARY_NAME` | Binary name (default `mpus`) |
| Variable | `DEPLOY_SERVICE_NAME` | systemd unit name (default `mpus`) |
| Variable | `DEPLOY_HEALTH_URL` | URL polled after restart, e.g. `https://mpus.booroong.online/health` |

The deploy user needs passwordless `sudo systemctl restart <service>`. The current VM
already has this (`sana ALL=(ALL) NOPASSWD:ALL` from cloud-init). No extra software is
needed on the server: the binary is self-contained (ONNX Runtime is linked statically, no
`.so` to ship), and the deploy is a plain scp and a systemctl call, so git is not required
on the VM.

### Manual deploy

Same steps the workflow runs, by hand:

```shell
cargo build --release
scp target/release/mpus <user>@<host>:/home/sana/mpus/mpus.new
ssh <user>@<host> 'cd /home/sana/mpus && cp -f mpus mpus.bak && mv -f mpus.new mpus \
  && chmod +x mpus && sudo systemctl restart mpus && sleep 3 && systemctl is-active mpus'
```

### Moving to a new server

1. Copy `deploy/mpus.service` to `/etc/systemd/system/mpus.service` and edit the paths if
   the app directory differs.
2. Copy `deploy/nginx-mpus.conf` to `/etc/nginx/sites-enabled/mpus`, replace the domain,
   and issue certs with `sudo certbot --nginx -d your.domain`.
3. Create the app directory, drop in the binary, `.env`, `models/`, and
   `fcm-service-account.json`.
4. Update the `DEPLOY_*` repository variables and the `SSH_PRIVATE_KEY` secret.

## License

This project is licensed under the **PolyForm Noncommercial License 1.0.0**. You may read,
run, and modify it for any noncommercial purpose. Commercial use is not allowed.

See the [LICENSE](./LICENSE) file for the exact terms.
