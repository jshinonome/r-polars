% Generated by roxygen2: do not edit by hand
% Please edit documentation in R/expr__expr.R
\name{Expr_lit}
\alias{Expr_lit}
\title{Create a literal value}
\usage{
Expr_lit(x)
}
\arguments{
\item{x}{A vector of any length}
}
\value{
Expr
}
\description{
Create a literal value
}
\details{
\code{pl$lit(NULL)} translates into a polars \code{null}.
}
\examples{
# scalars to literal, explicit `pl$lit(42)` implicit `+ 2`
pl$col("some_column") / pl$lit(42) + 2

# vector to literal explicitly via Series and back again
# R vector to expression and back again
pl$select(pl$lit(pl$Series(1:4)))$to_list()[[1L]]

# r vector to literal and back r vector
pl$lit(1:4)$to_r()

# r vector to literal to dataframe
pl$select(pl$lit(1:4))

# r vector to literal to Series
pl$lit(1:4)$to_series()

# vectors to literal implicitly
(pl$lit(2) + 1:4) / 4:1
}
