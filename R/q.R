Q$new <- function(
    host,
    port,
    user = "",
    password = "",
    enable_tls = FALSE) {
  .Call(wrap__Q__new, host, port, user, password, enable_tls)
}

Q$sync <- function(expr, ...) .Call(wrap__Q__execute, self, expr, list(...))

Q$async <- function(expr, ...) .Call(wrap__Q__execute_async, self, expr, list(...))
