# Keyless deploys from GitHub Actions (Workload Identity Federation). No service-account keys exist.

resource "google_iam_workload_identity_pool" "github" {
  workload_identity_pool_id = "${var.name}-github"
  display_name              = "GitHub Actions"
  depends_on                = [google_project_service.this]
}

resource "google_iam_workload_identity_pool_provider" "github" {
  workload_identity_pool_id          = google_iam_workload_identity_pool.github.workload_identity_pool_id
  workload_identity_pool_provider_id = "github-oidc"
  display_name                       = "GitHub OIDC"

  attribute_mapping = {
    "google.subject"       = "assertion.sub"
    "attribute.repository" = "assertion.repository"
    "attribute.ref"        = "assertion.ref"
  }

  # Only this repository (and, if set, only this ref) can mint tokens.
  attribute_condition = var.github_deploy_ref == "" ? "assertion.repository == '${var.github_repository}'" : "assertion.repository == '${var.github_repository}' && assertion.ref == '${var.github_deploy_ref}'"

  oidc {
    issuer_uri = "https://token.actions.githubusercontent.com"
  }
}

resource "google_service_account" "deployer" {
  account_id   = "${var.name}-deployer"
  display_name = "Astra deployer (GitHub Actions)"
}

resource "google_service_account_iam_member" "deployer_wif" {
  service_account_id = google_service_account.deployer.name
  role               = "roles/iam.workloadIdentityUser"
  member             = "principalSet://iam.googleapis.com/${google_iam_workload_identity_pool.github.name}/attribute.repository/${var.github_repository}"
}

# Push images.
resource "google_artifact_registry_repository_iam_member" "deployer" {
  repository = google_artifact_registry_repository.this.name
  location   = google_artifact_registry_repository.this.location
  role       = "roles/artifactregistry.writer"
  member     = google_service_account.deployer.member
}

# Roll out new relay revisions (only this service).
resource "google_cloud_run_v2_service_iam_member" "deployer" {
  name     = google_cloud_run_v2_service.relay.name
  location = google_cloud_run_v2_service.relay.location
  role     = "roles/run.developer"
  member   = google_service_account.deployer.member
}

# Deploying a revision that runs as the relay account requires acting as it.
resource "google_service_account_iam_member" "deployer_act_as_relay" {
  service_account_id = google_service_account.relay.name
  role               = "roles/iam.serviceAccountUser"
  member             = google_service_account.deployer.member
}

# Upload the web build.
resource "google_storage_bucket_iam_member" "deployer" {
  bucket = google_storage_bucket.web.name
  role   = "roles/storage.objectAdmin"
  member = google_service_account.deployer.member
}

# Invalidate the CDN after a web deploy — nothing else on the load balancer.
resource "google_project_iam_custom_role" "cdn_invalidator" {
  role_id     = replace("${var.name}_cdn_invalidator", "-", "_")
  title       = "Astra CDN invalidator"
  permissions = ["compute.urlMaps.get", "compute.urlMaps.invalidateCache", "compute.globalOperations.get"]
}

resource "google_project_iam_member" "deployer_cdn" {
  project = var.project_id
  role    = google_project_iam_custom_role.cdn_invalidator.id
  member  = google_service_account.deployer.member
}
