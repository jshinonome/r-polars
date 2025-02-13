% Generated by roxygen2: do not edit by hand
% Please edit documentation in R/expr__datetime.R
\name{ExprDT_day}
\alias{ExprDT_day}
\alias{(Expr)$dt$day}
\title{Day}
\usage{
ExprDT_day()
}
\value{
Expr of day as UInt32
}
\description{
Extract day from underlying Date representation.
Applies to Date and Datetime columns.
Returns the day of month starting from 1.
The return value ranges from 1 to 31. (The last day of month differs by months.)
}
\examples{
df = pl$DataFrame(
  date = pl$date_range(
    as.Date("2020-12-25"),
    as.Date("2021-1-05"),
    interval = "1d",
    time_zone = "GMT",
    eager = TRUE
  )
)
df$with_columns(
  pl$col("date")$dt$day()$alias("day")
)
}
\keyword{ExprDT}
