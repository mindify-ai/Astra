# One global HTTPS load balancer: web_domain → bucket + CDN, sync_domain → Cloud Run relay.

resource "google_compute_global_address" "lb" {
  name = "${var.name}-lb"
}

resource "google_compute_region_network_endpoint_group" "relay" {
  name                  = "${var.name}-relay-neg"
  region                = var.region
  network_endpoint_type = "SERVERLESS"
  cloud_run {
    service = google_cloud_run_v2_service.relay.name
  }
}

resource "google_compute_security_policy" "relay" {
  name        = "${var.name}-relay"
  description = "Throttle abusive clients before they reach the relay."
  type        = "CLOUD_ARMOR"

  rule {
    action      = "throttle"
    priority    = 1000
    description = "Per-IP request rate"
    match {
      versioned_expr = "SRC_IPS_V1"
      config {
        src_ip_ranges = ["*"]
      }
    }
    rate_limit_options {
      conform_action = "allow"
      exceed_action  = "deny(429)"
      enforce_on_key = "IP"
      rate_limit_threshold {
        count        = var.rate_limit_per_minute
        interval_sec = 60
      }
    }
  }

  rule {
    action   = "allow"
    priority = 2147483647
    match {
      versioned_expr = "SRC_IPS_V1"
      config {
        src_ip_ranges = ["*"]
      }
    }
    description = "Default allow"
  }
}

resource "google_compute_backend_service" "relay" {
  name                  = "${var.name}-relay"
  protocol              = "HTTPS"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  security_policy       = google_compute_security_policy.relay.id

  backend {
    group = google_compute_region_network_endpoint_group.relay.id
  }

  log_config {
    enable      = true
    sample_rate = 0.1
  }
}

resource "google_compute_url_map" "https" {
  name            = "${var.name}-lb"
  default_service = google_compute_backend_bucket.web.id

  host_rule {
    hosts        = [var.sync_domain]
    path_matcher = "relay"
  }

  path_matcher {
    name            = "relay"
    default_service = google_compute_backend_service.relay.id
  }
}

resource "google_compute_managed_ssl_certificate" "this" {
  name = "${var.name}-cert-${substr(sha1(join(",", [var.web_domain, var.sync_domain])), 0, 6)}"

  managed {
    domains = [var.web_domain, var.sync_domain]
  }

  lifecycle {
    create_before_destroy = true
  }
}

resource "google_compute_target_https_proxy" "this" {
  name             = "${var.name}-https"
  url_map          = google_compute_url_map.https.id
  ssl_certificates = [google_compute_managed_ssl_certificate.this.id]
}

resource "google_compute_global_forwarding_rule" "https" {
  name                  = "${var.name}-https"
  target                = google_compute_target_https_proxy.this.id
  ip_address            = google_compute_global_address.lb.id
  port_range            = "443"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  labels                = local.labels
}

# Plain HTTP only redirects to HTTPS.
resource "google_compute_url_map" "http_redirect" {
  name = "${var.name}-http-redirect"
  default_url_redirect {
    https_redirect         = true
    redirect_response_code = "MOVED_PERMANENTLY_DEFAULT"
    strip_query            = false
  }
}

resource "google_compute_target_http_proxy" "redirect" {
  name    = "${var.name}-http"
  url_map = google_compute_url_map.http_redirect.id
}

resource "google_compute_global_forwarding_rule" "http" {
  name                  = "${var.name}-http"
  target                = google_compute_target_http_proxy.redirect.id
  ip_address            = google_compute_global_address.lb.id
  port_range            = "80"
  load_balancing_scheme = "EXTERNAL_MANAGED"
  labels                = local.labels
}
