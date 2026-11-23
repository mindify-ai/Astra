variable "project_id" {
  description = "GCP project that hosts Astra."
  type        = string
}

variable "region" {
  description = "Region for Cloud Run, Cloud SQL and Artifact Registry. Pick the one closest to your users."
  type        = string
  default     = "asia-east1"
}

variable "name" {
  description = "Prefix for every resource name."
  type        = string
  default     = "astra"

  validation {
    condition     = can(regex("^[a-z][a-z0-9-]{1,18}[a-z0-9]$", var.name))
    error_message = "Use 3-20 lowercase letters, digits or hyphens, starting with a letter."
  }
}

variable "web_domain" {
  description = "Domain for the web app, e.g. app.example.com."
  type        = string
}

variable "sync_domain" {
  description = "Domain for the sync relay (WebSocket), e.g. sync.example.com."
  type        = string
}

variable "dns_zone_name" {
  description = "Existing Cloud DNS managed zone to create A records in. Leave empty to manage DNS elsewhere."
  type        = string
  default     = ""
}

variable "github_repository" {
  description = "owner/repo allowed to deploy through Workload Identity Federation."
  type        = string
  default     = "mindify-ai/Astra"
}

variable "github_deploy_ref" {
  description = "Git ref allowed to deploy (empty allows any ref of the repository)."
  type        = string
  default     = "refs/heads/main"
}

variable "relay_image" {
  description = "Initial relay image. CI deploys new revisions afterwards; Terraform ignores later image changes."
  type        = string
  default     = "us-docker.pkg.dev/cloudrun/container/hello"
}

variable "relay_min_instances" {
  description = "Always-on relay instances. Keep >= 1 so WebSocket clients never wait for a cold start."
  type        = number
  default     = 1
}

variable "relay_max_instances" {
  description = <<-EOT
    Maximum relay instances. The relay fans ops out in-process, so every client of a workspace must reach the
    same instance: keep this at 1 until cross-instance fan-out (NATS) ships, then raise it.
  EOT
  type        = number
  default     = 1
}

variable "relay_cpu" {
  description = "vCPU per relay instance."
  type        = string
  default     = "1"
}

variable "relay_memory" {
  description = "Memory per relay instance."
  type        = string
  default     = "512Mi"
}

variable "github_oauth_client_id" {
  description = "GitHub OAuth app client id for device-flow sign-in (public value; empty disables GitHub sign-in)."
  type        = string
  default     = ""
}

variable "gitlab_oauth_client_id" {
  description = "GitLab OAuth app client id for device-flow sign-in (empty disables GitLab sign-in)."
  type        = string
  default     = ""
}

variable "db_tier" {
  description = "Cloud SQL machine tier."
  type        = string
  default     = "db-custom-1-3840"
}

variable "db_high_availability" {
  description = "Regional (failover) Cloud SQL instance. Recommended for production; doubles database cost."
  type        = bool
  default     = false
}

variable "db_disk_gb" {
  description = "Initial Cloud SQL disk size; it grows automatically."
  type        = number
  default     = 20
}

variable "rate_limit_per_minute" {
  description = "Requests per minute allowed from one IP before Cloud Armor throttles it."
  type        = number
  default     = 600
}

variable "deletion_protection" {
  description = "Protect the database and relay from accidental destroy. Turn off only to tear down a test environment."
  type        = bool
  default     = true
}

variable "labels" {
  description = "Labels added to every resource that supports them."
  type        = map(string)
  default     = {}
}
