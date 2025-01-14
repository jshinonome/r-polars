% Generated by roxygen2: do not edit by hand
% Please edit documentation in R/functions__lazy.R
\name{pl_sum}
\alias{pl_sum}
\title{Compute sum in one or several columns}
\usage{
pl_sum(...)
}
\arguments{
\item{...}{One or several elements. Each element can be:
\itemize{
\item Series or Expr
\item string, that is parsed as columns
\item numeric, that is parsed as literal
}}
}
\value{
Expr
}
\description{
This is syntactic sugar for \code{pl$col(...)$sum()}.
}
\examples{
# column as string
pl$DataFrame(iris)$select(pl$sum("Petal.Width"))

# column as Expr (prefer pl$col("Petal.Width")$sum())
pl$DataFrame(iris)$select(pl$sum(pl$col("Petal.Width")))

df = pl$DataFrame(a = 1:2, b = 3:4, c = 5:6)

# Compute sum in several columns
df$with_columns(pl$sum("*"))
}
\keyword{Expr_new}
