test_that("q serialization", {
  q = Q("localhost", 1800)
  # RPolarsSeries
  expect_true(
    q$sync("{x ~ 5.0 * 1+til 5}", pl$Series((1:5) * 5, "my_series"))
  )
  # RPolarsDataFrame
  df = pl$DataFrame(
    sym = c("", "a", "apple"),
    qty = 1:3
  )
  expect_true(
    q$sync('{x ~ ([]sym:("";enlist "a";"apple");qty:1 2 3i)}', df)
  )
  t = as.POSIXlt("2024-02-13 07:33:32.929106")
  # POSIXlt
  expect_true(
    q$sync("{x ~ 2024.02.13D07:33:32.929106}", t)
  )
  t = as.POSIXct("2024-02-13 07:33:32.929106")
  # POSIXct
  expect_true(
    q$sync("{x ~ 2024.02.13D07:33:32.929106}", t)
  )
  # difftime
  expect_true(
    q$sync("{x ~ 100D}", as.difftime(100, units = "days"))
  )
  # Date
  expect_true(
    q$sync("{x ~ 2024.02.13}", as.Date("2024-02-13"))
  )
  # raw
  expect_true(
    q$sync('{x ~ "test"}', charToRaw("test"))
  )
  # NA
  expect_true(
    q$sync("{x ~ (::)}", NA)
  )
  # logical
  expect_true(
    q$sync("{x ~ 1b}", TRUE)
  )
  # integer
  expect_true(
    q$sync("{x ~ 123i}", as.integer(123))
  )
  # number
  expect_true(
    q$sync("{x ~ 123.}", 123)
  )
  # string
  expect_true(
    q$sync("{x ~ `test}", "test")
  )
  # Null
  expect_true(
    q$sync("{x ~ (::)}", NULL)
  )
  # list -> dict
  expect_true(
    q$sync(
      "{x ~ `start`end!2023.01.01 2023.02.13}",
      list(start = as.Date("2023-01-01"), end = as.Date("2023-02-13"))
    )
  )
})

test_that("q deserialization", {
  q = Q("localhost", 1800)
  # RPolarsSeries
  expect_identical(
    q$sync("5.0 * 1+til 5")$to_r(), pl$Series((1:5) * 5, "float")$to_r()
  )
  # RPolarsDataFrame
  df = pl$DataFrame(
    sym = c("", "a", "apple"),
    qty = 1:3
  )
  expect_identical(
    as.data.frame(q$sync('([]sym:("";enlist "a";"apple");qty:1 2 3i)')), as.data.frame(df)
  )
  # POSIXlt
  t = as.POSIXlt("2024-02-13 07:33:32.929106")
  expect_identical(
    q$sync("2024.02.13D07:33:32.929106"), t
  )
  # POSIXct
  t = as.POSIXct("2024-02-13 07:33:32.929106")
  expect_identical(
    q$sync("2024.02.13D07:33:32.929106"), t
  )
  # difftime
  expect_identical(
    as.numeric(q$sync("100D"), units = "secs"), as.numeric(as.difftime(100, units = "days"), units = "secs")
  )
  # Date
  expect_identical(
    q$sync("2024.02.13"), as.Date("2024-02-13")
  )
  # raw
  expect_identical(
    q$sync('"test"'), "test"
  )
  # logical
  expect_identical(
    q$sync("1b"), TRUE
  )
  # integer
  expect_identical(
    q$sync("123i"), as.integer(123)
  )
  # number
  expect_identical(
    q$sync("123."), 123
  )
  # string
  expect_identical(
    q$sync("`test"), "test"
  )
  # Null
  expect_identical(
    q$sync("(::)"), NULL
  )
  # list -> dict
  expect_error(
    q$sync(
      "`start`end!2023.01.01 2023.02.13"
    ),
    "Not support k type 11 in dictionary"
  )
})
