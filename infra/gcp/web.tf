# The web SaaS (apps/web): a static PWA in a bucket, served through the load balancer with Cloud CDN.

resource "google_storage_bucket" "web" {
  name                        = "${var.project_id}-${var.name}-web"
  location                    = upper(var.region)
  storage_class               = "STANDARD"
  uniform_bucket_level_access = true
  force_destroy               = !var.deletion_protection
  labels                      = local.labels

  website {
    main_page_suffix = "index.html"
    # Client-side routes (e.g. /missions/m-014) fall back to the app shell.
    not_found_page = "index.html"
  }

  cors {
    origin          = ["https://${var.web_domain}"]
    method          = ["GET", "HEAD"]
    response_header = ["Content-Type"]
    max_age_seconds = 3600
  }
}

# Objects are public: the app shell and hashed assets contain no data. All data flows through the relay.
resource "google_storage_bucket_iam_member" "web_public" {
  bucket = google_storage_bucket.web.name
  role   = "roles/storage.objectViewer"
  member = "allUsers"
}

resource "google_compute_backend_bucket" "web" {
  name        = "${var.name}-web"
  bucket_name = google_storage_bucket.web.name
  enable_cdn  = true

  cdn_policy {
    cache_mode        = "USE_ORIGIN_HEADERS"
    negative_caching  = false
    serve_while_stale = 86400
  }

  compression_mode = "AUTOMATIC"

  custom_response_headers = [
    "Strict-Transport-Security: max-age=31536000; includeSubDomains",
    "X-Content-Type-Options: nosniff",
    "Referrer-Policy: strict-origin-when-cross-origin",
    "Permissions-Policy: camera=(), microphone=(), geolocation=()",
  ]
}
