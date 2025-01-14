% Generated by roxygen2: do not edit by hand
% Please edit documentation in R/expr__expr.R
\name{Expr_add}
\alias{Expr_add}
\alias{+.RPolarsExpr}
\title{Add two expressions}
\usage{
Expr_add(other)

\method{+}{RPolarsExpr}(e1, e2)
}
\arguments{
\item{other}{Literal or object that can be converted to a literal}

\item{e1}{Expr only}

\item{e2}{Expr or anything that can be converted to a literal}
}
\value{
Expr
}
\description{
The RHS can either be an Expr or an object that can be converted to a literal
(e.g an integer).
}
\examples{
pl$lit(5) + 10
pl$lit(5) + pl$lit(10)
pl$lit(5)$add(pl$lit(10))
+pl$lit(5) # unary use resolves to same as pl$lit(5)
}
