% Generated by roxygen2: do not edit by hand
% Please edit documentation in R/functions__lazy.R
\name{pl_median}
\alias{pl_median}
\title{pl$median}
\usage{
pl_median(...)
}
\arguments{
\item{...}{One or several elements:
\itemize{
\item Series: Take mean value in \code{Series}
\item DataFrame or LazyFrame: Take mean value of each column
\item character vector: parsed as column names
\item NULL: expression to take mean column of a context.
}}
}
\value{
Expr or median value of input Series
}
\description{
Depending on the input type this function does different things:
}
\examples{

df = pl$DataFrame(
  a = c(1, 8, 3),
  b = c(4, 5, 2),
  c = c("foo", "bar", "foo")
)
df$select(pl$median("a"))

df$select(pl$median("a", "b"))

}
\keyword{Expr_new}
