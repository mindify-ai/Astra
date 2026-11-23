output "load_balancer_ip" {
  description = "Point web_domain and sync_domain A records here (done for you when dns_zone_name is set)."
  value       = google_compute_global_address.lb.address
}

output "web_url" {
  value = "https://${var.web_domain}"
}

output "relay_url" {
  description = "Build the web app with VITE_ASTRA_RELAY_URL set to this."
  value       = "https://${var.sync_domain}"
}

output "web_bucket" {
  value = google_storage_bucket.web.name
}

output "image_repository" {
  description = "Push relay images here."
  value       = "${google_artifact_registry_repository.this.location}-docker.pkg.dev/${var.project_id}/${google_artifact_registry_repository.this.repository_id}"
}

output "relay_service" {
  value = google_cloud_run_v2_service.relay.name
}

output "url_map" {
  description = "For CDN invalidation after web deploys."
  value       = google_compute_url_map.https.name
}

output "database_instance" {
  value = google_sql_database_instance.this.connection_name
}

output "github_actions_variables" {
  description = "Set these as GitHub repository variables (Settings → Secrets and variables → Actions → Variables)."
  value = {
    GCP_PROJECT_ID   = var.project_id
    GCP_REGION       = var.region
    GCP_WIF_PROVIDER = google_iam_workload_identity_pool_provider.github.name
    GCP_DEPLOY_SA    = google_service_account.deployer.email
    GCP_IMAGE_REPO   = "${google_artifact_registry_repository.this.location}-docker.pkg.dev/${var.project_id}/${google_artifact_registry_repository.this.repository_id}"
    GCP_RELAY        = google_cloud_run_v2_service.relay.name
    GCP_WEB_BUCKET   = google_storage_bucket.web.name
    GCP_URL_MAP      = google_compute_url_map.https.name
    ASTRA_RELAY_URL  = "https://${var.sync_domain}"
  }
}
