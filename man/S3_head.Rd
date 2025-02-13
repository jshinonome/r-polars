% Generated by roxygen2: do not edit by hand
% Please edit documentation in R/s3_methods.R
\name{head.RPolarsDataFrame}
\alias{head.RPolarsDataFrame}
\alias{head.RPolarsLazyFrame}
\alias{tail.RPolarsDataFrame}
\alias{tail.RPolarsLazyFrame}
\title{Return the first or the last \code{n} parts of an object}
\usage{
\method{head}{RPolarsDataFrame}(x, n = 6L, ...)

\method{head}{RPolarsLazyFrame}(x, n = 6L, ...)

\method{tail}{RPolarsDataFrame}(x, n = 6L, ...)

\method{tail}{RPolarsLazyFrame}(x, n = 6L, ...)
}
\arguments{
\item{x}{A polars object}

\item{n}{An integer vector of length 1.
Note that negative values are not supported for if \code{x} is a \link[=LazyFrame_class]{LazyFrame}.}

\item{...}{Ignored}
}
\value{
A polars object of the same class as \code{x}
}
\description{
They are equivalent to \verb{$head()} and \verb{$tail()} methods.
}
\examples{
df = pl$DataFrame(foo = 1:5, bar = 6:10, ham = letters[1:5])
lf = df$lazy()

head(df, 2)
tail(df, 2)

head(lf, 2)
tail(lf, 2)

head(df, -2)
tail(df, -2)
}
\seealso{
\itemize{
\item \code{\link[=DataFrame_head]{<DataFrame>$head()}}
\item \code{\link[=LazyFrame_head]{<LazyFrame>$head()}}
\item \code{\link[=DataFrame_tail]{<DataFrame>$tail()}}
\item \code{\link[=LazyFrame_tail]{<LazyFrame>$tail()}}
\item \code{\link[=LazyFrame_fetch]{<LazyFrame>$fetch()}}
}
}
