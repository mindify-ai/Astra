# Astra on Google Cloud (Terraform)

This module deploys the hosted parts of Astra:

- the **sync relay** (`crates/astra-server`);
- the **web app** (`apps/web`).

Desktop and phone apps ship through the release workflow. Agents never run here: they run on desktops or on
`astrad` hosts, and their keys stay there.

```
                        ┌─────────────── Global external HTTPS load balancer ───────────────┐
  app.example.com  ───► │ managed TLS cert · HTTP→HTTPS redirect                            │
                        │   web_domain  → backend bucket (Cloud CDN) → GCS bucket (PWA)      │
  sync.example.com ───► │   sync_domain → Cloud Armor (per-IP rate limit) → serverless NEG  │
                        └──────────────────────────────────────┬────────────────────────────┘
                                                               ▼
                         Cloud Run "astra-relay" (WebSockets, 60 min timeout, always-on CPU)
                               │ Direct VPC egress (private ranges)        │ Secret Manager
                               ▼                                           │ DATABASE_URL, JWT secret
                         Cloud SQL Postgres 17 (private IP only, TLS, PITR backups)

  GitHub Actions ──(Workload Identity Federation, no keys)──► Artifact Registry · Cloud Run · GCS · CDN
```

## What it creates

| File          | Resources                                                                                                                             |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `main.tf`     | Required Google APIs                                                                                                                  |
| `network.tf`  | VPC, Cloud Run subnet, private services access for Cloud SQL                                                                          |
| `database.tf` | Cloud SQL Postgres 17: private IP, `ENCRYPTED_ONLY`, point-in-time recovery, 14 backups, optional HA                                  |
| `secrets.tf`  | Secret Manager: database URL and JWT signing secret (generated)                                                                       |
| `registry.tf` | Artifact Registry for images, with a cleanup policy                                                                                   |
| `relay.tf`    | Cloud Run v2 service, its least-privilege service account, and health probes (`/readyz` startup, `/healthz` liveness)                 |
| `web.tf`      | Website bucket (SPA fallback to `index.html`), CDN backend bucket, security headers                                                   |
| `lb.tf`       | Global IP, managed certificate for both domains, URL map, Cloud Armor throttle, HTTP redirect                                         |
| `dns.tf`      | A records, only when `dns_zone_name` is set                                                                                           |
| `ci.tf`       | Workload Identity pool for this repo and a deployer account scoped to the registry, this service, the web bucket and CDN invalidation |

## First deploy

You need `gcloud` and Terraform ≥ 1.9, plus Owner (or equivalent) on the project.

**1. State bucket (once per project).** State contains the generated database password, so keep this bucket private:

```bash
gcloud storage buckets create gs://YOUR_PROJECT-astra-tfstate --project YOUR_PROJECT --location asia-east1 --uniform-bucket-level-access --public-access-prevention
```

```bash
gcloud storage buckets update gs://YOUR_PROJECT-astra-tfstate --versioning
```

**2. Configure.** Copy the examples, then edit `backend.hcl` and `terraform.tfvars`:

```bash
cp backend.hcl.example backend.hcl && cp terraform.tfvars.example terraform.tfvars
```

**3. Apply:**

```bash
terraform init -backend-config=backend.hcl
```

```bash
terraform apply
```

The first apply runs the relay with a placeholder image. The first CI deploy replaces it.

**4. DNS.**

- If you didn't set `dns_zone_name`, point `web_domain` and `sync_domain` A records at `terraform output load_balancer_ip`.
- The managed certificate becomes active once DNS resolves. That usually takes 15–60 minutes.

**5. GitHub variables.**

- Copy each entry of `terraform output github_actions_variables` into the repository's Actions **variables**.
- None of them are secrets, and no keys are involved.
- Pushing to `main` then runs `.github/workflows/deploy-gcp.yml`, which builds and rolls out the relay and the web app.

## Operating

- **Scaling.**
  - The relay fans ops out in-process, so `relay_max_instances` stays at **1** until cross-instance fan-out
    (NATS) ships.
  - Scale up first: raise `relay_cpu` and `relay_memory`. One instance handles up to 1,000 concurrent WebSockets
    (`max_instance_request_concurrency`).
  - `relay_min_instances = 1` keeps it warm. Cloud Run ends each connection after 60 minutes, and clients
    reconnect and catch up from their cursor.
- **Database.**
  - Set `db_high_availability = true` for production failover.
  - Backups run daily, with 7 days of point-in-time recovery.
- **Secrets.**
  - To rotate the JWT secret, taint `random_password.jwt`, apply, then redeploy. Existing sessions sign in again.
  - Database credentials rotate the same way with `random_password.db`.
- **Logs.**
  - The relay logs JSON to Cloud Logging.
  - The load balancer samples 10% of requests.
  - Cloud SQL Query Insights is on.
- **Teardown (test environments).** Set `deletion_protection = false`, apply, then `terraform destroy`.

## Organisation-policy notes

- The web bucket grants `allUsers` read access. It only serves the static app shell, and all data goes through the
  relay. If _Domain restricted sharing_ or _public access prevention_ is enforced, exempt this bucket or serve
  the app from Firebase Hosting instead.
- The relay uses `invoker_iam_disabled`, so no `allUsers` IAM binding is needed. Every connection is
  authenticated by the relay's own JWT check, and ingress is limited to the load balancer.

## Checks

These run in CI without GCP credentials, using mocked providers in `tests/plan.tftest.hcl`:

```bash
terraform fmt -check -recursive && terraform init -backend=false && terraform validate && terraform test
```
