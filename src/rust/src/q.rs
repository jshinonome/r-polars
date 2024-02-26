use std::cmp::{max, min};

use crate::{rdataframe::RPolarsDataFrame, series::RPolarsSeries};
use extendr_api::prelude::*;

use kola::types::{Dict, K};
use polars::export::chrono::{Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use polars_core::export::chrono::Datelike;
// use kola::types::{Dict, K};

pub struct Q {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub enable_tls: bool,
    q: kola::q::Q,
}

#[extendr]
impl Q {
    pub(crate) fn new(host: &str, port: u16, user: &str, password: &str, enable_tls: bool) -> Self {
        Q {
            host: host.to_string(),
            port,
            user: user.to_string(),
            password: password.to_string(),
            enable_tls,
            q: kola::q::Q::new(host, port, user, password, enable_tls),
        }
    }

    fn execute(&mut self, expr: &str, args: List) -> Robj {
        let k = match self.q.execute(expr, &cast_to_k_vec(args)) {
            Ok(k) => k,
            Err(e) => throw_r_error(e.to_string()),
        };
        match k {
            K::Bool(k) => k.into(),
            K::Guid(k) => k.to_string().into(),
            K::Byte(k) => k.into(),
            K::Short(k) => k.into(),
            K::Int(k) => k.into(),
            K::Long(k) => k.into(),
            K::Real(k) => k.into(),
            K::Float(k) => k.into(),
            K::Char(k) => (k as char).to_string().into(),
            K::Symbol(k) => k.into(),
            K::String(k) => k.into(),
            K::DateTime(k) => {
                let ms = k.timestamp_micros();
                let secs = ms as f64 / 1000000.0;
                R!("as.POSIXct({{secs}})").unwrap()
            }
            K::Date(k) => {
                let mut days = k.num_days_from_ce() as i64 - 719163;
                days = min(days, 2932532);
                days = max(days, -719162);
                R!("as.Date({{days}})").unwrap()
            }
            K::Time(k) => k.to_string().into(),
            K::Duration(k) => {
                let secs = k.num_microseconds().unwrap() as f64 / 1e6;
                R!("as.difftime({{secs}}, units = \"secs\")").unwrap()
            }
            K::Series(k) => RPolarsSeries::from(k).into(),
            K::DataFrame(k) => RPolarsDataFrame::from(k).into(),
            K::None(_) => ().into(),
            K::Dict(_) => throw_r_error("No plan to support deserializing dictionary"),
        }
    }

    fn execute_async(&mut self, expr: &str, args: List) -> () {
        match self.q.execute_async(expr, &cast_to_k_vec(args)) {
            Ok(_) => (),
            Err(e) => throw_r_error(format!("{:?}", e)),
        }
    }

    pub fn connect(&mut self) -> () {
        match self.q.connect() {
            Ok(_) => (),
            Err(e) => throw_r_error(format!("{:?}", e)),
        }
    }

    pub fn shutdown(&mut self) -> () {
        match self.q.shutdown() {
            Ok(_) => (),
            Err(e) => throw_r_error(format!("{:?}", e)),
        }
    }
}

fn cast_to_k_vec(list: List) -> Vec<K> {
    let mut vec: Vec<K> = Vec::with_capacity(list.len());
    for (_, obj) in list.into_iter() {
        match cast_to_k(obj) {
            Ok(obj) => vec.push(obj),
            Err(e) => throw_r_error(e.to_string()),
        };
    }
    vec
}

fn cast_to_k(any: Robj) -> Result<K> {
    if any.inherits("RPolarsSeries") {
        let series: ExternalPtr<RPolarsSeries> = any.try_into().unwrap();
        Ok(K::Series(series.0.clone()))
    } else if any.inherits("RPolarsDataFrame") {
        let df: ExternalPtr<RPolarsDataFrame> = any.try_into().unwrap();
        Ok(K::DataFrame(df.0.clone()))
    } else if any.inherits("POSIXlt") || any.inherits("POSIXct") {
        let datetime_str = R!("as.character({{any}})").unwrap().as_str().unwrap();
        Ok(K::DateTime(
            Utc.from_local_datetime(
                &NaiveDateTime::parse_from_str(datetime_str, "%Y-%m-%d %H:%M:%S%.f").unwrap(),
            )
            .unwrap(),
        ))
    } else if any.inherits("difftime") {
        let secs = R!("as.numeric({{any}}, units = \"secs\")")
            .unwrap()
            .as_real()
            .unwrap();
        Ok(K::Duration(Duration::microseconds((secs * 1e6) as i64)))
    } else if any.inherits("Date") {
        Ok(K::Date(
            NaiveDate::from_num_days_from_ce_opt(719163 + any.as_real().unwrap() as i32).unwrap(),
        ))
    } else if any.is_raw() {
        Ok(K::String(
            R!("rawToChar({{any}})")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
        ))
    } else if any.is_na() {
        Ok(K::None(0))
    } else if any.is_logical() {
        Ok(K::Bool(any.as_bool().unwrap()))
        // TODO: this heap allocs on failure
    } else if any.is_integer() {
        Ok(K::Int(any.as_integer().unwrap()))
    } else if any.is_number() {
        Ok(K::Float(any.as_real().unwrap()))
    } else if any.is_string() {
        Ok(K::Symbol(any.as_str().unwrap().into()))
    } else if any.is_null() {
        Ok(K::None(0))
    } else if any.is_list() || !any.is_vector_atomic() {
        let list = any.as_list().unwrap();
        let mut dict = Dict::with_capacity(any.len());
        for (k, v) in list.into_iter() {
            if k.is_empty() || k.is_na() {
                return Err(Error::Other(format!(
                    "kola: Not support list with null key as a dictionary - value {:?}",
                    v,
                )));
            }
            let v = cast_to_k(v)?;
            dict.set(k.to_string(), v).unwrap();
        }
        Ok(K::Dict(dict))
    } else if any.is_vector_atomic() {
        Err(Error::Other(
            "kola: not support serialize R vector, use polars series instead".to_string(),
        ))
    } else {
        Err(Error::Other(format!(
            "kola: Not supported R class {:?}",
            any.class(),
        )))
    }
}

extendr_module! {
    mod q;
    impl Q;
}
