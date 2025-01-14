% Generated by roxygen2: do not edit by hand
% Please edit documentation in R/expr__string.R
\name{ExprStr_to_datetime}
\alias{ExprStr_to_datetime}
\title{Convert a String column into a Datetime column}
\usage{
ExprStr_to_datetime(
  format = NULL,
  time_unit = NULL,
  time_zone = NULL,
  strict = TRUE,
  exact = TRUE,
  cache = TRUE,
  ambiguous = "raise"
)
}
\arguments{
\item{format}{Format to use for conversion. See \code{?strptime} for possible
values. Example: "\%Y-\%m-\%d \%H:\%M:\%S". If \code{NULL} (default), the format is
inferred from the data. Notice that time zone \verb{\%Z} is not supported and will
just ignore timezones. Numeric time zones like \verb{\%z} or \verb{\%:z}  are supported.}

\item{time_unit}{String (\code{"ns"}, \code{"us"}, \code{"ms"}) or integer.}

\item{time_zone}{String describing a timezone. If \code{NULL} (default), \verb{"GMT} is
used.}

\item{strict}{If \code{TRUE} (default), raise an error if a single string cannot
be parsed. If \code{FALSE}, parsing failure will produce a polars \code{null}.}

\item{exact}{If \code{TRUE} (default), require an exact format match. Otherwise,
allow the format to match anywhere in the target string.}

\item{cache}{Use a cache of unique, converted dates to apply the datetime
conversion.}

\item{ambiguous}{Determine how to deal with ambiguous datetimes:
\itemize{
\item \code{"raise"} (default): raise
\item \code{"earliest"}: use the earliest datetime
\item \code{"latest"}: use the latest datetime
}}
}
\value{
Expr
}
\description{
Convert a String column into a Datetime column
}
\examples{
pl$DataFrame(str_date = c("2009-01-02 01:00", "2009-01-03 02:00", "2009-1-4 3:00"))$
  with_columns(datetime = pl$col("str_date")$str$to_datetime(strict = FALSE))
}
