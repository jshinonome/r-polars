# kola

a R [Polars](https://rpolars.github.io/) Interface to kdb+/q

## Basic Data Type Map

### Deserialization

#### Atom

| k type      | n   | size | r type      | note                        |
| ----------- | --- | ---- | ----------- | --------------------------- |
| `boolean`   | 1   | 1    | `logical`   |                             |
| `guid`      | 2   | 16   | `character` |                             |
| `byte`      | 4   | 1    | `integer`   |                             |
| `short`     | 5   | 2    | `integer`   |                             |
| `int`       | 6   | 4    | `integer`   |                             |
| `long`      | 7   | 8    | `numberic`  |                             |
| `real`      | 8   | 4    | `float`     |                             |
| `float`     | 9   | 8    | `float`     |                             |
| `char`      | 10  | 1    | `character` |                             |
| `string`    | 10  | 1    | `character` |                             |
| `symbol`    | 11  | \*   | `character` |                             |
| `timestamp` | 12  | 8    | `character` |                             |
| `month`     | 13  | 4    | `-`         |                             |
| `date`      | 14  | 4    | `Date`      | 0001.01.01 - 9999.12.31     |
| `datetime`  | 15  | 8    | `character` |                             |
| `timespan`  | 16  | 8    | `difftime`  |                             |
| `minute`    | 17  | 4    | `character` | 00:00 - 23:59               |
| `second`    | 18  | 4    | `character` | 00:00:00 - 23:59:59         |
| `time`      | 19  | 4    | `character` | 00:00:00.000 - 23:59:59.999 |

#### Composite Data Type

| k type           | n   | size | python type              |
| ---------------- | --- | ---- | ------------------------ |
| `boolean list`   | 1   | 1    | `pl$Boolean`             |
| `guid list`      | 2   | 16   | `pl$List(pl$Binary(16))` |
| `byte list`      | 4   | 1    | `pl$Uint8`               |
| `short list`     | 5   | 2    | `pl$Int16`               |
| `int list`       | 6   | 4    | `pl$Int32`               |
| `long list`      | 7   | 8    | `pl$Int64`               |
| `real list`      | 8   | 4    | `pl$Float32`             |
| `float list`     | 9   | 8    | `pl$Float64`             |
| `char list`      | 10  | 1    | `pl$Utf8`                |
| `string list`    | 10  | 1    | `pl$Utf8`                |
| `symbol list`    | 11  | \*   | `pl$Categorical`         |
| `timestamp list` | 12  | 8    | `pl$Datetime`            |
| `month list`     | 13  | 4    | `-`                      |
| `date list`      | 14  | 4    | `pl$Date`                |
| `datetime list`  | 15  | 8    | `pl$Datetime`            |
| `timespan list`  | 16  | 8    | `pl$Duration`            |
| `minute list`    | 17  | 4    | `pl$Time`                |
| `second list`    | 18  | 4    | `pl$Time`                |
| `time list`      | 19  | 4    | `pl$Time`                |
| `table`          | 98  | \*   | `pl$DataFrame`           |
| `dictionary`     | 99  | \*   | `-`                      |
| `keyed table`    | 99  | \*   | `pl$DataFrame`           |

> performance is impacted by converting guid to string, deserialize the uuid to 16 fixed binary list

> real/float 0n is mapped to Polars null not NaN

### Serialization

#### Basic Data Type

| r type     | k type      | note                    |
| ---------- | ----------- | ----------------------- |
| `logical`  | `boolean`   |                         |
| `integer`  | `int`       |                         |
| `float`    | `float`     |                         |
| `str`      | `symbol`    |                         |
| `raw`      | `string`    |                         |
| `POSIXlt`  | `timestamp` |                         |
| `POSIXct`  | `timestamp` |                         |
| `date`     | `date`      | 0001.01.01 - 9999.12.31 |
| `difftime` | `timespan`  |                         |

#### Dictionary, Series and DataFrame

| r type                   | k type    |
| ------------------------ | --------- |
| `list`                   | dict      |
| `pl$Boolean`             | boolean   |
| `pl$List(pl$Binary(16))` | guid      |
| `pl$Uint8`               | byte      |
| `pl$Int16`               | short     |
| `pl$Int32`               | int       |
| `pl$Int64`               | long      |
| `pl$Float32`             | real      |
| `pl$Float64`             | float     |
| `pl$Utf8`                | char      |
| `pl$Categorical`         | symbol    |
| `pl$Datetime`            | timestamp |
| `pl$Date`                | date      |
| `pl$Datetime`            | datetime  |
| `pl$duration`            | timespan  |
| `pl$Time`                | time      |
| `pl$DataFrame`           | table     |

> Limited Support for dictionary as arguments, r list with `string` as keys and r `Basic Data Types` and `pl.Series` as values.

## Quick Start

#### Create a Connection

```r
library(polars)
q <- Q("localhost", 1800)
```

#### Connect(Optional)

Automatically connect when querying q process

```r
q$connect()
```

#### Disconnect

Automatically disconnect if any IO error

```r
q$disconnect()
```

#### String Query

```r
q$sync("select from trade where date=last date")
```

#### Functional Query

For functional query, `kola` supports R data types (logical, character, numeric, integer, POSIXlt, POSIXct, raw, NA, NULL), `RPolarsSeries`, `RPolarsDataFrame` and R list with string keys.

```r
q$sync(
    ".gw.query",
    "trade",
    list(
        syms = pl$Series(c("7203.T", "2226.T"), "syms"),
        startDate = as.Date("2023-01-01"),
        endDate = as.Date("2023-02-13")
    )
)
```

#### Send DataFrame

```r
# pl_df is a Polars DataFrame
q$sync("upsert", "alpha", pl_df)
```

#### Async Query

```r
# pl_df is a Polars DataFrame
q$async("upsert", "alpha", pl_df)
```

#### Polars Documentations

Refer to

- [Polars R Package](https://rpolars.github.io/)
- [API Reference](https://rpolars.github.io/reference_home.html)

#### Build Binary

```r
devtools::install(quick = TRUE)
devtools::test(stop_on_failure = TRUE)
devtools::build(binary = TRUE, args = c('--preclean'), path="/workspaces/r-polars")
```

#### Install Binary

```r
install.packages("polars_0.14.1_R_x86_64-pc-linux-gnu.tar.gz", repos = NULL, type="source", dependencies = TRUE)
```
