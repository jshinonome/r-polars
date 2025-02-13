% Generated by roxygen2: do not edit by hand
% Please edit documentation in R/dataframe__frame.R
\name{DataFrame_drop}
\alias{DataFrame_drop}
\title{Drop columns of a DataFrame}
\usage{
DataFrame_drop(columns)
}
\arguments{
\item{columns}{A character vector with the names of the column(s) to remove.}
}
\value{
DataFrame
}
\description{
Drop columns of a DataFrame
}
\examples{
pl$DataFrame(mtcars)$drop(c("mpg", "hp"))
}
\keyword{DataFrame}
