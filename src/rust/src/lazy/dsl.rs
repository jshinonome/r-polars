use crate::concurrent::RFnSignature;
use crate::rdatatype::{
    literal_to_any_value, new_rolling_cov_options, parse_fill_null_strategy, robj_to_timeunit,
    RPolarsDataType, RPolarsDataTypeVector,
};
use crate::robj_to;
use crate::rpolarserr::{
    polars_to_rpolars_err, rerr, rpolars_to_polars_err, RResult, Rctx, WithRctx,
};
use crate::series::RPolarsSeries;
use crate::utils::extendr_concurrent::{ParRObj, ThreadCom};
use crate::utils::extendr_helpers::robj_inherits;
use crate::utils::robj_to_rchoice;
use crate::utils::wrappers::null_to_opt;
use crate::utils::{r_error_list, r_ok_list, r_result_list, robj_to_binary_vec};
use crate::utils::{try_f64_into_i64, try_f64_into_u32, try_f64_into_usize};
use crate::CONFIG;
use extendr_api::{extendr, prelude::*, rprintln, Deref, DerefMut, Rinternals};
use pl::PolarsError as pl_error;
use pl::{
    BinaryNameSpaceImpl, Duration, DurationMethods, IntoSeries, RollingGroupOptions, SetOperation,
    StringNameSpaceImpl, TemporalMethods,
};
use polars::lazy::dsl;
use polars::prelude as pl;
use polars::prelude::SortOptions;
use std::ops::{Add, Div, Mul, Rem, Sub};
use std::result::Result;
pub type NameGenerator = pl::Arc<dyn Fn(usize) -> String + Send + Sync>;
use crate::rdatatype::robjs_to_ewm_options;

use crate::utils::r_expr_to_rust_expr;
use crate::utils::unpack_r_eval;
use smartstring::{LazyCompact, SmartString};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct RPolarsExpr(pub pl::Expr);

impl Deref for RPolarsExpr {
    type Target = pl::Expr;
    fn deref(&self) -> &pl::Expr {
        &self.0
    }
}

impl DerefMut for RPolarsExpr {
    fn deref_mut(&mut self) -> &mut pl::Expr {
        &mut self.0
    }
}

impl From<RPolarsExpr> for pl::Expr {
    fn from(x: RPolarsExpr) -> Self {
        x.0
    }
}

impl From<pl::Expr> for RPolarsExpr {
    fn from(expr: pl::Expr) -> Self {
        RPolarsExpr(expr)
    }
}

#[extendr]
impl RPolarsExpr {
    //constructors
    pub fn col(name: &str) -> Self {
        dsl::col(name).into()
    }

    //via col
    pub fn dtype_cols(dtypes: &RPolarsDataTypeVector) -> Self {
        dsl::dtype_cols(dtypes.dtv_to_vec()).into()
    }

    //via col
    pub fn cols(names: Vec<String>) -> Self {
        dsl::cols(names).into()
    }

    pub fn lit(robj: Robj) -> RResult<RPolarsExpr> {
        let rtype = robj.rtype();
        let rlen = robj.len();

        fn to_series_then_lit(robj: Robj) -> RResult<pl::Expr> {
            RPolarsSeries::any_robj_to_pl_series_result(robj)
                .map_err(polars_to_rpolars_err)
                .map(dsl::lit)
        }

        match (rtype, rlen) {
            (Rtype::Null, _) => Ok(dsl::lit(pl::NULL)),
            (Rtype::Raw, _) => Ok(dsl::lit(robj_to_binary_vec(robj)?)), // Raw in R is seen as a vector of bytes, in polars it is a Literal, not wrapped in a Series.
            (_, rlen) if rlen != 1 => to_series_then_lit(robj),
            (Rtype::List, _) => to_series_then_lit(robj),
            (_, _) if robj_inherits(&robj, ["POSIXct", "PTime", "Date"]) => {
                to_series_then_lit(robj)
            }

            (Rtype::Integers, 1) => {
                let opt_val = robj.as_integer();
                if let Some(val) = opt_val {
                    Ok(dsl::lit(val))
                } else if robj.is_na() {
                    Ok(dsl::lit(pl::NULL).cast(pl::DataType::Int32))
                } else {
                    unreachable!("internal error: could unexpectedly not handle this R value");
                }
            }
            (Rtype::Doubles, 1) if robj.inherits("integer64") => {
                let opt_val = robj.as_real();
                if let Some(val) = opt_val {
                    let x = val.to_bits() as i64;
                    if x == crate::utils::BIT64_NA_ENCODING {
                        Ok(dsl::lit(pl::NULL).cast(pl::DataType::Int64))
                    } else {
                        Ok(dsl::lit(x))
                    }
                } else {
                    unreachable!("internal error: could unexpectedly not handle this R value");
                }
            }
            (Rtype::Doubles, 1) => {
                let opt_val = robj.as_real();
                if let Some(val) = opt_val {
                    Ok(dsl::lit(val))
                } else if robj.is_na() {
                    Ok(dsl::lit(pl::NULL).cast(pl::DataType::Float64))
                } else {
                    unreachable!("internal error: could unexpectedly not handle this R value");
                }
            }
            (Rtype::Strings, 1) => {
                if robj.is_na() {
                    Ok(dsl::lit(pl::NULL).cast(pl::DataType::String))
                } else {
                    Ok(dsl::lit(robj.as_str().unwrap()))
                }
            }
            (Rtype::Logicals, 1) => {
                let opt_val = robj.as_bool();
                if let Some(val) = opt_val {
                    Ok(dsl::lit(val))
                } else if robj.is_na() {
                    Ok(dsl::lit(pl::NULL).cast(pl::DataType::Boolean))
                } else {
                    unreachable!("internal error: could unexpectedly not handle this R value");
                }
            }
            (Rtype::ExternalPtr, 1) => match () {
                _ if robj.inherits("RPolarsSeries") => {
                    let s: RPolarsSeries =
                        unsafe { &mut *robj.external_ptr_addr::<RPolarsSeries>() }.clone();
                    Ok(pl::lit(s.0))
                }

                _ if robj.inherits("RPolarsExpr") => {
                    let expr: RPolarsExpr =
                        unsafe { &mut *robj.external_ptr_addr::<RPolarsExpr>() }.clone();
                    Ok(expr.0)
                }

                _ if robj_inherits(&robj, ["RPolarsThen", "RPolarsChainedThen"]) => unpack_r_eval(
                    R!("polars:::result({{robj}}$otherwise(polars::pl$lit(NULL)))"),
                )
                .and_then(r_expr_to_rust_expr)
                .map(|expr| expr.0),

                _ if robj_inherits(&robj, ["RPolarsWhen", "RPolarsChainedWhen"]) => rerr()
                    .plain("Cannot use a When or ChainedWhen-statement as Expr without a $then()"),

                _ => rerr()
                    .bad_robj(&robj)
                    .plain("pl$lit() this ExternalPtr class is not currently supported"),
            },

            (_, _) => rerr().bad_robj(&robj).plain("unsupported R type "),
        }
        .map(RPolarsExpr)
        .when("constructing polars literal from Robj")
    }

    //comparison
    pub fn gt(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().gt(robj_to!(PLExpr, other)?).into())
    }

    pub fn gt_eq(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().gt_eq(robj_to!(PLExpr, other)?).into())
    }

    pub fn lt(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().lt(robj_to!(PLExpr, other)?).into())
    }

    pub fn lt_eq(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().lt_eq(robj_to!(PLExpr, other)?).into())
    }

    pub fn neq(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().neq(robj_to!(PLExpr, other)?).into())
    }

    pub fn neq_missing(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().neq_missing(robj_to!(PLExpr, other)?).into())
    }

    pub fn eq(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().eq(robj_to!(PLExpr, other)?).into())
    }

    pub fn eq_missing(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().eq_missing(robj_to!(PLExpr, other)?).into())
    }

    //conjunction
    fn and(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().and(robj_to!(PLExpr, other)?).into())
    }

    fn or(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().or(robj_to!(PLExpr, other)?).into())
    }

    //binary
    fn xor(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().xor(robj_to!(PLExpr, other)?).into())
    }

    //any not translated expr from expr/expr.py
    pub fn to_physical(&self) -> Self {
        self.0
            .clone()
            .map(
                |s| Ok(Some(s.to_physical_repr().into_owned())),
                pl::GetOutput::map_dtype(|dt| dt.to_physical()),
            )
            .with_fmt("to_physical")
            .into()
    }

    pub fn cast(&self, data_type: &RPolarsDataType, strict: bool) -> Self {
        let dt = data_type.0.clone();
        if strict {
            self.0.clone().strict_cast(dt)
        } else {
            self.0.clone().cast(dt)
        }
        .into()
    }

    pub fn sort(&self, descending: bool, nulls_last: bool) -> Self {
        self.clone()
            .0
            .sort_with(SortOptions {
                descending,
                nulls_last,
                multithreaded: true,
                maintain_order: false,
            })
            .into()
    }

    pub fn arg_sort(&self, descending: bool, nulls_last: bool) -> Self {
        self.clone()
            .0
            .arg_sort(SortOptions {
                descending,
                nulls_last,
                multithreaded: true,
                maintain_order: false,
            })
            .into()
    }

    pub fn top_k(&self, k: Robj) -> RResult<Self> {
        Ok(self.0.clone().top_k(robj_to!(PLExpr, k)?).into())
    }

    pub fn bottom_k(&self, k: Robj) -> RResult<Self> {
        Ok(self.0.clone().bottom_k(robj_to!(PLExpr, k)?).into())
    }

    pub fn arg_max(&self) -> Self {
        self.clone().0.arg_max().into()
    }
    pub fn arg_min(&self) -> Self {
        self.clone().0.arg_min().into()
    }

    //TODO expose searchSorted side options
    pub fn search_sorted(&self, element: &RPolarsExpr) -> Self {
        use pl::SearchSortedSide as Side;
        self.0
            .clone()
            .search_sorted(element.0.clone(), Side::Any)
            .into()
    }

    pub fn gather(&self, idx: Robj) -> RResult<Self> {
        Ok(self
            .clone()
            .0
            .gather(robj_to!(PLExpr, idx)?.cast(pl::DataType::Int64))
            .into())
    }

    pub fn sort_by(&self, by: Robj, descending: Robj) -> RResult<RPolarsExpr> {
        let expr = RPolarsExpr(self.clone().0.sort_by(
            robj_to!(VecPLExprCol, by)?,
            robj_to!(Vec, bool, descending)?,
        ));
        Ok(expr)
    }

    pub fn backward_fill(&self, limit: Nullable<f64>) -> Self {
        let lmt = null_to_opt(limit).map(|x| x as u32);
        self.clone().0.backward_fill(lmt).into()
    }

    pub fn forward_fill(&self, limit: Nullable<f64>) -> Self {
        let lmt = null_to_opt(limit).map(|x| x as u32);
        self.clone().0.forward_fill(lmt).into()
    }

    pub fn shift(&self, periods: Robj) -> RResult<Self> {
        Ok(self.clone().0.shift(robj_to!(PLExpr, periods)?).into())
    }

    pub fn shift_and_fill(&self, periods: Robj, fill_value: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .shift_and_fill(robj_to!(PLExpr, periods)?, robj_to!(PLExpr, fill_value)?)
            .into())
    }

    pub fn fill_null(&self, expr: Robj) -> RResult<Self> {
        Ok(self.0.clone().fill_null(robj_to!(PLExpr, expr)?).into())
    }

    pub fn fill_null_with_strategy(&self, strategy: Robj, limit: Robj) -> RResult<Self> {
        let strat = parse_fill_null_strategy(
            robj_to_rchoice(strategy)?.as_str(),
            robj_to!(Option, u32, limit)?,
        )?;
        let expr: pl::Expr = self
            .0
            .clone()
            .apply(
                move |s| s.fill_null(strat).map(Some),
                pl::GetOutput::same_type(),
            )
            .with_fmt("fill_null_with_strategy");

        Ok(RPolarsExpr(expr))
    }

    pub fn fill_nan(&self, expr: &RPolarsExpr) -> Self {
        self.0.clone().fill_nan(expr.0.clone()).into()
    }

    pub fn reverse(&self) -> Self {
        self.0.clone().reverse().into()
    }

    pub fn std(&self, ddof: Robj) -> RResult<Self> {
        Ok(self.clone().0.std(robj_to!(u8, ddof)?).into())
    }

    pub fn var(&self, ddof: Robj) -> RResult<Self> {
        Ok(self.clone().0.var(robj_to!(u8, ddof)?).into())
    }

    pub fn max(&self) -> Self {
        self.0.clone().max().into()
    }

    pub fn min(&self) -> Self {
        self.0.clone().min().into()
    }

    pub fn nan_min(&self) -> Self {
        self.clone().0.nan_min().into()
    }

    pub fn nan_max(&self) -> Self {
        self.clone().0.nan_max().into()
    }

    pub fn mean(&self) -> Self {
        self.0.clone().mean().into()
    }

    pub fn median(&self) -> Self {
        self.0.clone().median().into()
    }

    pub fn sum(&self) -> Self {
        self.0.clone().sum().into()
    }

    pub fn product(&self) -> Self {
        self.clone().0.product().into()
    }

    pub fn n_unique(&self) -> Self {
        self.0.clone().n_unique().into()
    }

    pub fn null_count(&self) -> Self {
        self.0.clone().null_count().into()
    }

    pub fn arg_unique(&self) -> Self {
        self.clone().0.arg_unique().into()
    }

    pub fn quantile(&self, quantile: Robj, interpolation: Robj) -> RResult<Self> {
        Ok(self
            .clone()
            .0
            .quantile(
                robj_to!(PLExpr, quantile)?,
                robj_to!(quantile_interpolation_option, interpolation)?,
            )
            .into())
    }

    pub fn filter(&self, predicate: &RPolarsExpr) -> RPolarsExpr {
        self.clone().0.filter(predicate.0.clone()).into()
    }

    pub fn explode(&self) -> RPolarsExpr {
        self.clone().0.explode().into()
    }
    pub fn flatten(&self) -> RPolarsExpr {
        //same as explode
        self.clone().0.explode().into()
    }

    pub fn gather_every(&self, n: Robj, offset: Robj) -> RResult<RPolarsExpr> {
        let n = robj_to!(usize, n).and_then(|n| match n {
            0 => rerr().bad_arg("n").bad_val("n can't be zero"),
            _ => Ok(n),
        })?;
        let offset = robj_to!(usize, offset)?;
        Ok(self
            .0
            .clone()
            .map(
                move |s: pl::Series| Ok(Some(s.gather_every(n, offset))),
                pl::GetOutput::same_type(),
            )
            .with_fmt("gather_every")
            .into())
    }

    pub fn hash(&self, seed: Robj, seed_1: Robj, seed_2: Robj, seed_3: Robj) -> RResult<Self> {
        Ok(RPolarsExpr(self.0.clone().hash(
            robj_to!(u64, seed)?,
            robj_to!(u64, seed_1)?,
            robj_to!(u64, seed_2)?,
            robj_to!(u64, seed_3)?,
        )))
    }

    pub fn reinterpret(&self, signed: bool) -> RPolarsExpr {
        use crate::utils::reinterpret;
        let function = move |s: pl::Series| reinterpret(&s, signed).map(Some);
        let dt = if signed {
            pl::DataType::Int64
        } else {
            pl::DataType::UInt64
        };
        self.clone()
            .0
            .map(function, pl::GetOutput::from_type(dt))
            .into()
    }

    pub fn interpolate(&self, method: Robj) -> RResult<RPolarsExpr> {
        Ok(self
            .clone()
            .0
            .interpolate(robj_to!(InterpolationMethod, method)?)
            .into())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rolling_min(
        &self,
        window_size: Robj,
        weights: Robj,
        min_periods: Robj,
        center: Robj,
        by_null: Robj,
        closed_null: Robj,
        warn_if_unsorted: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .rolling_min(make_rolling_options(
                window_size,
                weights,
                min_periods,
                center,
                by_null,
                closed_null,
                warn_if_unsorted,
            )?)
            .into())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rolling_max(
        &self,
        window_size: Robj,
        weights: Robj,
        min_periods: Robj,
        center: Robj,
        by_null: Robj,
        closed_null: Robj,
        warn_if_unsorted: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .rolling_max(make_rolling_options(
                window_size,
                weights,
                min_periods,
                center,
                by_null,
                closed_null,
                warn_if_unsorted,
            )?)
            .into())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rolling_mean(
        &self,
        window_size: Robj,
        weights: Robj,
        min_periods: Robj,
        center: Robj,
        by_null: Robj,
        closed_null: Robj,
        warn_if_unsorted: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .rolling_mean(make_rolling_options(
                window_size,
                weights,
                min_periods,
                center,
                by_null,
                closed_null,
                warn_if_unsorted,
            )?)
            .into())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rolling_sum(
        &self,
        window_size: Robj,
        weights: Robj,
        min_periods: Robj,
        center: Robj,
        by_null: Robj,
        closed_null: Robj,
        warn_if_unsorted: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .rolling_sum(make_rolling_options(
                window_size,
                weights,
                min_periods,
                center,
                by_null,
                closed_null,
                warn_if_unsorted,
            )?)
            .into())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rolling_std(
        &self,
        window_size: Robj,
        weights: Robj,
        min_periods: Robj,
        center: Robj,
        by_null: Robj,
        closed_null: Robj,
        warn_if_unsorted: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .rolling_std(make_rolling_options(
                window_size,
                weights,
                min_periods,
                center,
                by_null,
                closed_null,
                warn_if_unsorted,
            )?)
            .into())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rolling_var(
        &self,
        window_size: Robj,
        weights: Robj,
        min_periods: Robj,
        center: Robj,
        by_null: Robj,
        closed_null: Robj,
        warn_if_unsorted: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .rolling_var(make_rolling_options(
                window_size,
                weights,
                min_periods,
                center,
                by_null,
                closed_null,
                warn_if_unsorted,
            )?)
            .into())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rolling_median(
        &self,
        window_size: Robj,
        weights: Robj,
        min_periods: Robj,
        center: Robj,
        by_null: Robj,
        closed_null: Robj,
        warn_if_unsorted: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .rolling_median(make_rolling_options(
                window_size,
                weights,
                min_periods,
                center,
                by_null,
                closed_null,
                warn_if_unsorted,
            )?)
            .into())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rolling_quantile(
        &self,
        quantile: Robj,
        interpolation: Robj,
        window_size: Robj,
        weights: Robj,
        min_periods: Robj,
        center: Robj,
        by: Robj,
        closed: Robj,
        warn_if_unsorted: Robj,
    ) -> RResult<Self> {
        let options = pl::RollingOptions {
            window_size: pl::Duration::parse(robj_to!(str, window_size)?),
            weights: robj_to!(Option, Vec, f64, weights)?,
            min_periods: robj_to!(usize, min_periods)?,
            center: robj_to!(bool, center)?,
            by: robj_to!(Option, String, by)?,
            closed_window: robj_to!(Option, ClosedWindow, closed)?,
            warn_if_unsorted: robj_to!(bool, warn_if_unsorted)?,
            fn_params: None,
        };
        let quantile = robj_to!(f64, quantile)?;
        let interpolation = robj_to!(quantile_interpolation_option, interpolation)?;

        Ok(self
            .0
            .clone()
            .rolling_quantile(interpolation, quantile, options)
            .into())
    }

    pub fn rolling_skew(&self, window_size_f: f64, bias: bool) -> List {
        use pl::*;
        let expr = try_f64_into_usize(window_size_f).map(|ws| {
            RPolarsExpr(
                self.0
                    .clone()
                    .rolling_map_float(ws, move |ca| ca.clone().into_series().skew(bias).unwrap()),
            )
        });
        r_result_list(expr)
    }

    pub fn abs(&self) -> Self {
        self.0.clone().abs().into()
    }

    fn rank(&self, method: Robj, descending: Robj, seed: Robj) -> RResult<Self> {
        let options = pl::RankOptions {
            method: robj_to!(RankMethod, method)?,
            descending: robj_to!(bool, descending)?,
        };
        let seed = robj_to!(Option, u64, seed)?;
        Ok(self.0.clone().rank(options, seed).into())
    }

    fn diff(&self, n_float: Robj, null_behavior: Robj) -> RResult<RPolarsExpr> {
        Ok(RPolarsExpr(self.0.clone().diff(
            robj_to!(i64, n_float)?,
            robj_to!(new_null_behavior, null_behavior)?,
        )))
    }

    fn pct_change(&self, n_float: Robj) -> RResult<Self> {
        Ok(RPolarsExpr(
            self.0.clone().pct_change(robj_to!(PLExpr, n_float)?),
        ))
    }

    fn skew(&self, bias: bool) -> Self {
        self.0.clone().skew(bias).into()
    }
    fn kurtosis(&self, fisher: bool, bias: bool) -> Self {
        self.0.clone().kurtosis(fisher, bias).into()
    }

    //Note clip is implemented a bit different that py-polars
    //instead of PyValue -> AnyValue , it goes Robj -> Literal Expression -> AnyValue
    pub fn clip(&self, min: Robj, max: Robj) -> RResult<Self> {
        let av_min = robj_to!(PLExpr, min)?;
        let av_max = robj_to!(PLExpr, max)?;
        Ok(RPolarsExpr(self.0.clone().clip(av_min, av_max)))
    }

    pub fn clip_min(&self, min: Robj) -> RResult<Self> {
        let av_min = robj_to!(PLExpr, min)?;
        Ok(RPolarsExpr(self.0.clone().clip_min(av_min)))
    }

    pub fn clip_max(&self, max: Robj) -> RResult<Self> {
        let av_max = robj_to!(PLExpr, max)?;
        Ok(RPolarsExpr(self.0.clone().clip_max(av_max)))
    }

    pub fn lower_bound(&self) -> Self {
        self.0.clone().lower_bound().into()
    }

    pub fn upper_bound(&self) -> Self {
        self.0.clone().upper_bound().into()
    }

    pub fn sign(&self) -> Self {
        self.clone().0.sign().into()
    }

    pub fn sin(&self) -> Self {
        self.clone().0.sin().into()
    }

    pub fn cos(&self) -> Self {
        self.clone().0.cos().into()
    }

    pub fn tan(&self) -> Self {
        self.clone().0.tan().into()
    }

    pub fn arcsin(&self) -> Self {
        self.clone().0.arcsin().into()
    }

    pub fn arccos(&self) -> Self {
        self.clone().0.arccos().into()
    }

    pub fn arctan(&self) -> Self {
        self.clone().0.arctan().into()
    }

    pub fn sinh(&self) -> Self {
        self.clone().0.sinh().into()
    }

    pub fn cosh(&self) -> Self {
        self.clone().0.cosh().into()
    }

    pub fn tanh(&self) -> Self {
        self.clone().0.tanh().into()
    }

    pub fn arcsinh(&self) -> Self {
        self.clone().0.arcsinh().into()
    }

    pub fn arccosh(&self) -> Self {
        self.clone().0.arccosh().into()
    }

    pub fn arctanh(&self) -> Self {
        self.clone().0.arctanh().into()
    }

    pub fn reshape(&self, dims: Vec<f64>) -> List {
        let dims_result: Result<Vec<i64>, String> = dims
            .iter()
            .map(|x| try_f64_into_i64(*x).map_err(String::from))
            .collect();
        let expr_result = dims_result
            .map(|dims| RPolarsExpr(self.0.clone().reshape(&dims[..])))
            .map_err(|err| format!("reshape: {}", err));
        r_result_list(expr_result)
    }

    pub fn shuffle(&self, seed: Robj) -> RResult<Self> {
        Ok(self.0.clone().shuffle(robj_to!(Option, u64, seed)?).into())
    }

    pub fn sample_n(
        &self,
        n: Robj,
        with_replacement: Robj,
        shuffle: Robj,
        seed: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .sample_n(
                robj_to!(PLExpr, n)?,
                robj_to!(bool, with_replacement)?,
                robj_to!(bool, shuffle)?,
                robj_to!(Option, u64, seed)?,
            )
            .into())
    }

    pub fn sample_frac(
        &self,
        frac: Robj,
        with_replacement: Robj,
        shuffle: Robj,
        seed: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .sample_frac(
                robj_to!(PLExpr, frac)?,
                robj_to!(bool, with_replacement)?,
                robj_to!(bool, shuffle)?,
                robj_to!(Option, u64, seed)?,
            )
            .into())
    }

    pub fn ewm_mean(
        &self,
        alpha: Robj,
        adjust: Robj,
        min_periods: Robj,
        ignore_nulls: Robj,
    ) -> RResult<Self> {
        let options = robjs_to_ewm_options(alpha, adjust, r!(false), min_periods, ignore_nulls)?;
        Ok(self.0.clone().ewm_mean(options).into())
    }

    pub fn ewm_std(
        &self,
        alpha: Robj,
        adjust: Robj,
        bias: Robj,
        min_periods: Robj,
        ignore_nulls: Robj,
    ) -> RResult<Self> {
        let options = robjs_to_ewm_options(alpha, adjust, bias, min_periods, ignore_nulls)?;
        Ok(self.0.clone().ewm_std(options).into())
    }

    pub fn ewm_var(
        &self,
        alpha: Robj,
        adjust: Robj,
        bias: Robj,
        min_periods: Robj,
        ignore_nulls: Robj,
    ) -> RResult<Self> {
        let options = robjs_to_ewm_options(alpha, adjust, bias, min_periods, ignore_nulls)?;
        Ok(self.0.clone().ewm_var(options).into())
    }

    pub fn extend_constant(&self, value: Robj, n: Robj) -> RResult<Self> {
        let av = robj_to!(LiteralValue, value).and_then(literal_to_any_value)?;
        let n = robj_to!(usize, n)?;
        Ok(RPolarsExpr(
            self.0
                .clone()
                .apply(
                    move |s| {
                        //swap owned inline string to str as only supported and if swapped here life time is long enough
                        let av = match &av {
                            pl::AnyValue::StringOwned(x) => pl::AnyValue::String(x.as_str()),
                            x => x.clone(),
                        };
                        s.extend_constant(av, n).map(Some)
                    },
                    pl::GetOutput::same_type(),
                )
                .with_fmt("extend"),
        ))
    }

    pub fn rep(&self, n: f64, rechunk: bool) -> List {
        match try_f64_into_usize(n) {
            Err(err) => r_error_list(format!("rep: arg n invalid, {}", err)),
            Ok(n) => r_ok_list(RPolarsExpr(
                self.0
                    .clone()
                    .apply(
                        move |s| {
                            if s.len() == 1 {
                                Ok(Some(s.new_from_index(0, n)))
                            } else {
                                RPolarsSeries(s).rep_impl(n, rechunk).map(|s| Some(s.0))
                            }
                        },
                        pl::GetOutput::same_type(),
                    )
                    .with_fmt("rep"),
            )),
        }
    }

    pub fn value_counts(&self, sort: bool, parallel: bool) -> Self {
        self.0.clone().value_counts(sort, parallel).into()
    }

    pub fn unique_counts(&self) -> Self {
        self.0.clone().unique_counts().into()
    }

    pub fn entropy(&self, base: f64, normalize: bool) -> Self {
        self.0.clone().entropy(base, normalize).into()
    }

    fn cumulative_eval(&self, expr: &RPolarsExpr, min_periods: f64, parallel: bool) -> List {
        use pl::*;
        r_result_list(try_f64_into_usize(min_periods).map(|min_p| {
            RPolarsExpr(
                self.0
                    .clone()
                    .cumulative_eval(expr.0.clone(), min_p, parallel),
            )
        }))
    }

    pub fn implode(&self) -> Self {
        self.clone().0.implode().into()
    }

    pub fn shrink_dtype(&self) -> Self {
        self.0.clone().shrink_dtype().into()
    }

    pub fn peak_min(&self) -> Self {
        self.0.clone().peak_min().into()
    }

    pub fn peak_max(&self) -> Self {
        self.0.clone().peak_max().into()
    }

    pub fn replace(
        &self,
        old: Robj,
        new: Robj,
        default: Robj,
        return_dtype: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .replace(
                robj_to!(PLExpr, old)?,
                robj_to!(PLExpr, new)?,
                robj_to!(Option, PLExpr, default)?,
                robj_to!(Option, PLPolarsDataType, return_dtype)?,
            )
            .into())
    }

    pub fn rle(&self) -> RResult<Self> {
        Ok(self.0.clone().rle().into())
    }

    pub fn rle_id(&self) -> RResult<Self> {
        Ok(self.0.clone().rle_id().into())
    }

    //arr/list methods

    fn list_len(&self) -> Self {
        self.0.clone().list().len().into()
    }

    pub fn list_contains(&self, other: &RPolarsExpr) -> RPolarsExpr {
        self.0.clone().list().contains(other.0.clone()).into()
    }

    fn list_max(&self) -> Self {
        self.0.clone().list().max().into()
    }

    fn list_min(&self) -> Self {
        self.0.clone().list().min().into()
    }

    fn list_sum(&self) -> Self {
        self.0.clone().list().sum().with_fmt("list.sum").into()
    }

    fn list_mean(&self) -> Self {
        self.0.clone().list().mean().with_fmt("list.mean").into()
    }

    fn list_sort(&self, descending: bool) -> Self {
        self.0
            .clone()
            .list()
            .sort(SortOptions {
                descending,
                ..Default::default()
            })
            .with_fmt("list.sort")
            .into()
    }

    fn list_reverse(&self) -> Self {
        self.0
            .clone()
            .list()
            .reverse()
            .with_fmt("list.reverse")
            .into()
    }

    fn list_unique(&self) -> Self {
        self.0
            .clone()
            .list()
            .unique()
            .with_fmt("list.unique")
            .into()
    }

    fn list_gather(&self, index: Robj, null_on_oob: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .list()
            .take(robj_to!(PLExprCol, index)?, robj_to!(bool, null_on_oob)?)
            .into())
    }

    fn list_get(&self, index: &RPolarsExpr) -> Self {
        self.0.clone().list().get(index.clone().0).into()
    }

    fn list_join(&self, separator: Robj, ignore_nulls: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .list()
            .join(robj_to!(PLExpr, separator)?, robj_to!(bool, ignore_nulls)?)
            .into())
    }

    fn list_arg_min(&self) -> Self {
        self.0.clone().list().arg_min().into()
    }

    fn list_arg_max(&self) -> Self {
        self.0.clone().list().arg_max().into()
    }

    fn list_diff(&self, n: Robj, null_behavior: Robj) -> RResult<Self> {
        Ok(RPolarsExpr(self.0.clone().list().diff(
            robj_to!(i64, n)?,
            robj_to!(new_null_behavior, null_behavior)?,
        )))
    }

    fn list_shift(&self, periods: Robj) -> RResult<Self> {
        Ok(RPolarsExpr(
            self.0.clone().list().shift(robj_to!(PLExpr, periods)?),
        ))
    }

    fn list_slice(&self, offset: &RPolarsExpr, length: Nullable<&RPolarsExpr>) -> Self {
        let length = match null_to_opt(length) {
            Some(i) => i.0.clone(),
            None => dsl::lit(i64::MAX),
        };
        self.0.clone().list().slice(offset.0.clone(), length).into()
    }

    fn list_eval(&self, expr: &RPolarsExpr, parallel: bool) -> Self {
        use pl::*;
        self.0.clone().list().eval(expr.0.clone(), parallel).into()
    }

    fn list_to_struct(
        &self,
        n_field_strategy: Robj,
        fields: Robj,
        upper_bound: Robj,
    ) -> RResult<Self> {
        let width_strat = robj_to!(ListToStructWidthStrategy, n_field_strategy)?;
        let fields = robj_to!(Option, Robj, fields)?.map(|robj| {
            let par_fn: ParRObj = robj.into();
            let f: Arc<(dyn Fn(usize) -> SmartString<LazyCompact> + Send + Sync + 'static)> =
                pl::Arc::new(move |idx: usize| {
                    let thread_com = ThreadCom::from_global(&CONFIG);
                    thread_com.send(RFnSignature::FnF64ToString(par_fn.clone(), idx as f64));
                    let s = thread_com.recv().unwrap_string();
                    let s: SmartString<LazyCompact> = s.into();
                    s
                });
            f
        });
        let ub = robj_to!(usize, upper_bound)?;
        Ok(RPolarsExpr(self.0.clone().list().to_struct(
            width_strat,
            fields,
            ub,
        )))
    }

    fn list_all(&self) -> Self {
        self.0.clone().list().all().into()
    }

    fn list_any(&self) -> Self {
        self.0.clone().list().any().into()
    }

    fn list_set_operation(&self, other: Robj, operation: Robj) -> RResult<Self> {
        let other = robj_to!(PLExprCol, other)?;
        let operation = robj_to!(SetOperation, operation)?;
        let e = self.0.clone().list();
        Ok(match operation {
            SetOperation::Intersection => e.set_intersection(other),
            SetOperation::Difference => e.set_difference(other),
            SetOperation::Union => e.union(other),
            SetOperation::SymmetricDifference => e.set_symmetric_difference(other),
        }
        .into())
    }

    // array methods

    fn arr_max(&self) -> Self {
        self.0.clone().arr().max().into()
    }

    fn arr_min(&self) -> Self {
        self.0.clone().arr().min().into()
    }

    fn arr_sum(&self) -> Self {
        self.0.clone().arr().sum().into()
    }

    // TODO: implement those in 0.38.0. They were wrongly included in the changelog of 0.37.0
    // https://github.com/pola-rs/polars/issues/14355
    // fn arr_std(&self, ddof: u8) -> Self {
    //     self.0.clone().arr().std(ddof).into()
    // }

    // fn arr_var(&self, ddof: u8) -> Self {
    //     self.0.clone().arr().var(ddof).into()
    // }

    // fn arr_median(&self) -> Self {
    //     self.0.clone().arr().median().into()
    // }

    fn arr_unique(&self, maintain_order: bool) -> Self {
        if maintain_order {
            self.0.clone().arr().unique_stable().into()
        } else {
            self.0.clone().arr().unique().into()
        }
    }

    fn arr_to_list(&self) -> Self {
        self.0.clone().arr().to_list().into()
    }

    fn arr_all(&self) -> Self {
        self.0.clone().arr().all().into()
    }

    fn arr_any(&self) -> Self {
        self.0.clone().arr().any().into()
    }

    fn arr_sort(&self, descending: bool, nulls_last: bool) -> Self {
        self.0
            .clone()
            .arr()
            .sort(SortOptions {
                descending,
                nulls_last,
                ..Default::default()
            })
            .into()
    }

    fn arr_reverse(&self) -> Self {
        self.0.clone().arr().reverse().into()
    }

    fn arr_arg_min(&self) -> Self {
        self.0.clone().arr().arg_min().into()
    }

    fn arr_arg_max(&self) -> Self {
        self.0.clone().arr().arg_max().into()
    }

    fn arr_get(&self, index: Robj) -> RResult<Self> {
        Ok(self.0.clone().arr().get(robj_to!(PLExprCol, index)?).into())
    }

    fn arr_join(&self, separator: Robj, ignore_nulls: bool) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .arr()
            .join(robj_to!(PLExpr, separator)?, ignore_nulls)
            .into())
    }

    fn arr_contains(&self, other: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .arr()
            .contains(robj_to!(PLExpr, other)?)
            .into())
    }

    fn arr_count_matches(&self, expr: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .arr()
            .count_matches(robj_to!(PLExprCol, expr)?)
            .into())
    }

    // TODO: implement those in 0.38.0. They were wrongly included in the changelog of 0.37.0
    // https://github.com/pola-rs/polars/issues/14355
    // fn arr_to_struct(&self, fields: Robj, upper_bound: Robj) -> RResult<Self> {
    //     let fields = robj_to!(Option, Robj, fields)?.map(|robj| {
    //         let par_fn: ParRObj = robj.into();
    //         let f: Arc<(dyn Fn(usize) -> SmartString<LazyCompact> + Send + Sync + 'static)> =
    //             pl::Arc::new(move |idx: usize| {
    //                 let thread_com = ThreadCom::from_global(&CONFIG);
    //                 thread_com.send(RFnSignature::FnF64ToString(par_fn.clone(), idx as f64));
    //                 let s = thread_com.recv().unwrap_string();
    //                 let s: SmartString<LazyCompact> = s.into();
    //                 s
    //             });
    //         f
    //     });
    //     Ok(RPolarsExpr(self.0.clone().arr().to_struct(fields)))
    // }

    // TODO: implement when bumping to rs-0.38.0
    // fn arr_shift(&self, n: Robj) -> RResult<Self> {
    //     Ok(self.0.clone().arr().shift(robj_to!(PLExprCol, n)?).into())
    // }

    // datetime methods

    pub fn dt_truncate(&self, every: Robj, offset: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .dt()
            .truncate(
                robj_to!(PLExpr, every)?,
                robj_to!(Option, pl_duration_string, offset)?.unwrap_or_else(|| "0ns".into()),
            )
            .into())
    }

    pub fn dt_round(&self, every: Robj, offset: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .dt()
            .round(
                robj_to!(pl_duration_string, every)?,
                robj_to!(Option, pl_duration_string, offset)?.unwrap_or_else(|| "0ns".into()),
            )
            .into())
    }

    pub fn dt_time(&self) -> RResult<Self> {
        Ok(self.0.clone().dt().time().into())
    }

    pub fn dt_combine(&self, time: Robj, tu: Robj) -> RResult<RPolarsExpr> {
        Ok(self
            .0
            .clone()
            .dt()
            .combine(robj_to!(PLExpr, time)?, robj_to!(timeunit, tu)?)
            .into())
    }

    pub fn dt_strftime(&self, fmt: &str) -> Self {
        //named strftime in py-polars
        self.0.clone().dt().strftime(fmt).into()
    }
    pub fn dt_year(&self) -> Self {
        //named year in py-polars
        self.clone().0.dt().year().into()
    }
    pub fn dt_iso_year(&self) -> Self {
        //named iso_year in py-polars
        self.clone().0.dt().iso_year().into()
    }

    pub fn dt_quarter(&self) -> Self {
        self.clone().0.dt().quarter().into()
    }
    pub fn dt_month(&self) -> Self {
        self.clone().0.dt().month().into()
    }
    pub fn dt_week(&self) -> Self {
        self.clone().0.dt().week().into()
    }
    pub fn dt_weekday(&self) -> Self {
        self.clone().0.dt().weekday().into()
    }
    pub fn dt_day(&self) -> Self {
        self.clone().0.dt().day().into()
    }
    pub fn dt_ordinal_day(&self) -> Self {
        self.clone().0.dt().ordinal_day().into()
    }
    pub fn dt_hour(&self) -> Self {
        self.clone().0.dt().hour().into()
    }
    pub fn dt_minute(&self) -> Self {
        self.clone().0.dt().minute().into()
    }
    pub fn dt_second(&self) -> Self {
        self.clone().0.dt().second().into()
    }
    pub fn dt_millisecond(&self) -> Self {
        self.clone().0.dt().millisecond().into()
    }
    pub fn dt_microsecond(&self) -> Self {
        self.clone().0.dt().microsecond().into()
    }
    pub fn dt_nanosecond(&self) -> Self {
        self.clone().0.dt().nanosecond().into()
    }

    pub fn timestamp(&self, tu: Robj) -> List {
        let res = robj_to_timeunit(tu)
            .map(|tu| RPolarsExpr(self.0.clone().dt().timestamp(tu)))
            .map_err(|err| format!("valid tu needed for timestamp: {}", err));
        r_result_list(res)
    }

    pub fn dt_epoch_seconds(&self) -> Self {
        self.clone()
            .0
            .map(
                |s| {
                    s.timestamp(pl::TimeUnit::Milliseconds)
                        .map(|ca| Some((ca / 1000).into_series()))
                },
                pl::GetOutput::from_type(pl::DataType::Int64),
            )
            .into()
    }

    pub fn dt_with_time_unit(&self, tu: Robj) -> RResult<RPolarsExpr> {
        Ok(RPolarsExpr(
            self.0.clone().dt().with_time_unit(robj_to!(timeunit, tu)?),
        ))
    }

    pub fn dt_cast_time_unit(&self, tu: Robj) -> RResult<RPolarsExpr> {
        Ok(RPolarsExpr(
            self.0.clone().dt().cast_time_unit(robj_to!(timeunit, tu)?),
        ))
    }

    pub fn dt_convert_time_zone(&self, tz: String) -> Self {
        self.0.clone().dt().convert_time_zone(tz).into()
    }

    pub fn dt_replace_time_zone(&self, tz: Nullable<String>, ambiguous: Robj) -> RResult<Self> {
        Ok(RPolarsExpr(self.0.clone().dt().replace_time_zone(
            tz.into_option(),
            robj_to!(PLExpr, ambiguous)?,
        )))
    }

    pub fn dt_total_days(&self) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .map(
                |s| Ok(Some(s.duration()?.days().into_series())),
                pl::GetOutput::from_type(pl::DataType::Int64),
            )
            .into())
    }
    pub fn dt_total_hours(&self) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .map(
                |s| Ok(Some(s.duration()?.hours().into_series())),
                pl::GetOutput::from_type(pl::DataType::Int64),
            )
            .into())
    }
    pub fn dt_total_minutes(&self) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .map(
                |s| Ok(Some(s.duration()?.minutes().into_series())),
                pl::GetOutput::from_type(pl::DataType::Int64),
            )
            .into())
    }
    pub fn dt_total_seconds(&self) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .map(
                |s| Ok(Some(s.duration()?.seconds().into_series())),
                pl::GetOutput::from_type(pl::DataType::Int64),
            )
            .into())
    }
    pub fn dt_total_milliseconds(&self) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .map(
                |s| Ok(Some(s.duration()?.milliseconds().into_series())),
                pl::GetOutput::from_type(pl::DataType::Int64),
            )
            .into())
    }
    pub fn dt_total_microseconds(&self) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .map(
                |s| Ok(Some(s.duration()?.microseconds().into_series())),
                pl::GetOutput::from_type(pl::DataType::Int64),
            )
            .into())
    }
    pub fn dt_total_nanoseconds(&self) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .map(
                |s| Ok(Some(s.duration()?.nanoseconds().into_series())),
                pl::GetOutput::from_type(pl::DataType::Int64),
            )
            .into())
    }

    pub fn dt_offset_by(&self, by: Robj) -> RResult<Self> {
        Ok(self.clone().0.dt().offset_by(robj_to!(PLExpr, by)?).into())
    }

    pub fn repeat_by(&self, by: &RPolarsExpr) -> Self {
        self.clone().0.repeat_by(by.0.clone()).into()
    }

    pub fn log10(&self) -> Self {
        self.0.clone().log(10.0).into()
    }

    // TODO contribute to polars
    // log/exp only takes float, whereas pow takes Into<Expr>
    // log takes a base value, whereas exp only is natural log

    pub fn log(&self, base: f64) -> Self {
        self.0.clone().log(base).into()
    }

    pub fn exp(&self) -> Self {
        self.0.clone().exp().into()
    }

    pub fn exclude(&self, columns: Vec<String>) -> Self {
        self.0.clone().exclude(columns).into()
    }

    pub fn exclude_dtype(&self, columns: &RPolarsDataTypeVector) -> Self {
        self.0.clone().exclude_dtype(columns.dtv_to_vec()).into()
    }

    pub fn alias(&self, s: &str) -> Self {
        self.0.clone().alias(s).into()
    }

    pub fn drop_nulls(&self) -> Self {
        self.0.clone().drop_nulls().into()
    }

    pub fn drop_nans(&self) -> Self {
        self.0.clone().drop_nans().into()
    }

    pub fn cum_sum(&self, reverse: Robj) -> RResult<Self> {
        Ok(self.0.clone().cum_sum(robj_to!(bool, reverse)?).into())
    }

    pub fn cum_prod(&self, reverse: Robj) -> RResult<Self> {
        Ok(self.0.clone().cum_prod(robj_to!(bool, reverse)?).into())
    }

    pub fn cum_min(&self, reverse: Robj) -> RResult<Self> {
        Ok(self.0.clone().cum_min(robj_to!(bool, reverse)?).into())
    }

    pub fn cum_max(&self, reverse: Robj) -> RResult<Self> {
        Ok(self.0.clone().cum_max(robj_to!(bool, reverse)?).into())
    }

    pub fn cum_count(&self, reverse: Robj) -> RResult<Self> {
        Ok(self.0.clone().cum_count(robj_to!(bool, reverse)?).into())
    }

    pub fn floor(&self) -> Self {
        self.0.clone().floor().into()
    }

    pub fn ceil(&self) -> Self {
        self.0.clone().ceil().into()
    }

    pub fn round(&self, decimals: f64) -> List {
        let res = try_f64_into_u32(decimals)
            .map_err(|err| format!("in round: {}", err))
            .map(|n| RPolarsExpr(self.0.clone().round(n)));
        r_result_list(res)
    }

    pub fn dot(&self, other: &RPolarsExpr) -> Self {
        self.0.clone().dot(other.0.clone()).into()
    }

    pub fn mode(&self) -> Self {
        self.0.clone().mode().into()
    }

    pub fn first(&self) -> Self {
        self.0.clone().first().into()
    }

    pub fn last(&self) -> Self {
        self.0.clone().last().into()
    }

    pub fn head(&self, n: Robj) -> RResult<Self> {
        Ok(self.0.clone().head(Some(robj_to!(usize, n)?)).into())
    }

    pub fn tail(&self, n: Robj) -> RResult<Self> {
        Ok(self.0.clone().tail(Some(robj_to!(usize, n)?)).into())
    }

    //chaining methods

    pub fn unique(&self) -> Self {
        self.0.clone().unique().into()
    }

    pub fn unique_stable(&self) -> Self {
        self.0.clone().unique_stable().into()
    }

    pub fn agg_groups(&self) -> Self {
        self.0.clone().agg_groups().into()
    }

    // boolean

    pub fn all(&self, drop_nulls: Robj) -> RResult<Self> {
        Ok(self.0.clone().all(robj_to!(bool, drop_nulls)?).into())
    }
    pub fn any(&self, drop_nulls: Robj) -> RResult<Self> {
        Ok(self.0.clone().any(robj_to!(bool, drop_nulls)?).into())
    }

    fn is_between(&self, lower: Robj, upper: Robj, closed: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .is_between(
                robj_to!(PLExprCol, lower)?,
                robj_to!(PLExprCol, upper)?,
                robj_to!(ClosedInterval, closed)?,
            )
            .into())
    }

    pub fn is_duplicated(&self) -> Self {
        self.clone().0.is_duplicated().into()
    }

    pub fn is_finite(&self) -> Self {
        self.0.clone().is_finite().into()
    }

    pub fn is_first_distinct(&self) -> Self {
        self.clone().0.is_first_distinct().into()
    }

    fn is_in(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().is_in(robj_to!(PLExpr, other)?).into())
    }

    pub fn is_infinite(&self) -> Self {
        self.0.clone().is_infinite().into()
    }

    pub fn is_last_distinct(&self) -> Self {
        self.clone().0.is_last_distinct().into()
    }

    pub fn is_nan(&self) -> Self {
        self.0.clone().is_nan().into()
    }
    pub fn is_not_null(&self) -> Self {
        self.0.clone().is_not_null().into()
    }

    pub fn is_not_nan(&self) -> Self {
        self.0.clone().is_not_nan().into()
    }
    pub fn is_null(&self) -> Self {
        self.0.clone().is_null().into()
    }

    pub fn is_unique(&self) -> Self {
        self.0.clone().is_unique().into()
    }
    pub fn not(&self) -> Self {
        self.0.clone().not().into()
    }

    pub fn count(&self) -> Self {
        self.0.clone().count().into()
    }

    pub fn len(&self) -> Self {
        self.0.clone().count().into()
    }

    pub fn slice(&self, offset: &RPolarsExpr, length: Nullable<&RPolarsExpr>) -> Self {
        let length = match null_to_opt(length) {
            Some(i) => i.0.clone(),
            None => dsl::lit(i64::MAX),
        };
        self.0.clone().slice(offset.0.clone(), length).into()
    }

    pub fn append(&self, other: &RPolarsExpr, upcast: bool) -> Self {
        self.0.clone().append(other.0.clone(), upcast).into()
    }

    pub fn rechunk(&self) -> Self {
        self.0
            .clone()
            .map(|s| Ok(Some(s.rechunk())), pl::GetOutput::same_type())
            .into()
    }

    //numeric

    pub fn add(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().add(robj_to!(PLExpr, other)?).into())
    }

    pub fn floor_div(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().floor_div(robj_to!(PLExpr, other)?).into())
    }

    pub fn rem(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().rem(robj_to!(PLExpr, other)?).into())
    }

    pub fn mul(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().mul(robj_to!(PLExpr, other)?).into())
    }

    pub fn sub(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().sub(robj_to!(PLExpr, other)?).into())
    }

    pub fn div(&self, other: Robj) -> RResult<Self> {
        Ok(self.0.clone().div(robj_to!(PLExpr, other)?).into())
    }

    pub fn pow(&self, exponent: Robj) -> RResult<Self> {
        Ok(self.0.clone().pow(robj_to!(PLExpr, exponent)?).into())
    }

    //expr      "funnies"
    pub fn over(&self, proto_exprs: &RPolarsProtoExprArray) -> Self {
        let ve = pra_to_vec(proto_exprs, "select");
        self.0.clone().over(ve).into()
    }

    pub fn print(&self) {
        rprintln!("{:#?}", self.0);
    }

    pub fn map_batches(&self, lambda: Robj, output_type: Robj, agg_list: Robj) -> RResult<Self> {
        // define closure how to request R code evaluated in main thread from a some polars sub thread
        let par_fn = ParRObj(lambda);
        let f = move |s: pl::Series| {
            let thread_com = ThreadCom::try_from_global(&CONFIG)
                .expect("polars was thread could not initiate ThreadCommunication to R");
            thread_com.send(RFnSignature::FnSeriesToSeries(par_fn.clone(), s));
            let s = thread_com.recv().unwrap_series();
            Ok(Some(s))
        };

        // set expected type of output from R function
        let ot = robj_to!(Option, PLPolarsDataType, output_type)?;
        let output_map = pl::GetOutput::map_field(move |fld| match ot {
            Some(ref dt) => pl::Field::new(fld.name(), dt.clone()),
            None => fld.clone(),
        });

        robj_to!(bool, agg_list)
            .map(|agg_list| {
                if agg_list {
                    self.clone().0.map_list(f, output_map)
                } else {
                    self.clone().0.map(f, output_map)
                }
            })
            .map(RPolarsExpr)
    }

    pub fn map_batches_in_background(
        &self,
        lambda: Robj,
        output_type: Robj,
        agg_list: Robj,
    ) -> RResult<Self> {
        let raw_func = crate::rbackground::serialize_robj(lambda).unwrap();

        let rbgfunc = move |s| {
            crate::RBGPOOL
                .rmap_series(raw_func.clone(), s)
                .map_err(rpolars_to_polars_err)?()
            .map_err(rpolars_to_polars_err)
            .map(Some)
        };

        let ot = robj_to!(Option, PLPolarsDataType, output_type)?;

        let output_map = pl::GetOutput::map_field(move |fld| match ot {
            Some(ref dt) => pl::Field::new(fld.name(), dt.clone()),
            None => fld.clone(),
        });

        robj_to!(bool, agg_list)
            .map(|agg_list| {
                if agg_list {
                    self.clone().0.map_list(rbgfunc, output_map)
                } else {
                    self.clone().0.map(rbgfunc, output_map)
                }
            })
            .map(RPolarsExpr)
    }

    pub fn map_elements_in_background(
        &self,
        lambda: Robj,
        output_type: Nullable<&RPolarsDataType>,
    ) -> Self {
        let raw_func = crate::rbackground::serialize_robj(lambda).unwrap();

        let rbgfunc = move |s| {
            crate::RBGPOOL
                .rmap_series(raw_func.clone(), s)
                .map_err(rpolars_to_polars_err)?()
            .map_err(rpolars_to_polars_err)
            .map(Some)
        };

        let ot = null_to_opt(output_type).map(|rdt| rdt.0.clone());

        let output_map = pl::GetOutput::map_field(move |fld| match ot {
            Some(ref dt) => pl::Field::new(fld.name(), dt.clone()),
            None => fld.clone(),
        });

        self.0.clone().apply(rbgfunc, output_map).into()
    }

    pub fn approx_n_unique(&self) -> Self {
        self.clone().0.approx_n_unique().into()
    }

    // name methods
    pub fn name_keep(&self) -> RResult<Self> {
        Ok(self.0.clone().name().keep().into())
    }

    fn name_suffix(&self, suffix: String) -> RResult<Self> {
        Ok(self.0.clone().name().suffix(suffix.as_str()).into())
    }

    fn name_prefix(&self, prefix: String) -> RResult<Self> {
        Ok(self.0.clone().name().prefix(prefix.as_str()).into())
    }

    fn name_to_lowercase(&self) -> RResult<Self> {
        Ok(self.0.clone().name().to_lowercase().into())
    }

    fn name_to_uppercase(&self) -> RResult<Self> {
        Ok(self.0.clone().name().to_uppercase().into())
    }

    pub fn name_map(&self, lambda: Robj) -> RResult<Self> {
        //find a way not to push lambda everytime to main thread handler
        //safety only accessed in main thread, can be temp owned by other threads
        let probj = ParRObj(lambda);
        //}

        // let f = move |name: &str| -> String {
        //     //acquire channel to R via main thread handler
        //     let thread_com = ThreadCom::from_global(&CONFIG);

        //     //place name in Series because current version of ThreadCom only speaks series
        //     use polars::prelude::NamedFrom;
        //     let s = pl::Series::new(name, &[0]);

        //     //send request to run in R
        //     thread_com.send((probj.clone(), s));

        //     //recieve answer
        //     let s = thread_com.recv();

        //     s.0.name().to_string()

        //     //wrap as series
        // };

        let f = move |name: &str| -> pl::PolarsResult<String> {
            let robj = probj.clone().0;
            let rfun = robj
                .as_function()
                .expect("internal error: this is not an R function");

            let newname_robj = rfun.call(pairlist!(name)).map_err(|err| {
                let es =
                    format!("in $name$map(): user function raised this error: {:?}", err).into();
                pl_error::ComputeError(es)
            })?;

            newname_robj
                .as_str()
                .ok_or_else(|| {
                    let es = "in $name$map(): R function return value was not a string"
                        .to_string()
                        .into();
                    pl_error::ComputeError(es)
                })
                .map(|str| str.to_string())
        };

        Ok(self.clone().0.name().map(f).into())
    }

    //string methods
    pub fn str_len_bytes(&self) -> Self {
        use pl::*;
        let function = |s: pl::Series| {
            let ca = s.str()?;
            Ok(Some(ca.str_len_bytes().into_series()))
        };
        self.clone()
            .0
            .map(function, pl::GetOutput::from_type(pl::DataType::UInt32))
            .with_fmt("str.len_bytes")
            .into()
    }

    pub fn str_len_chars(&self) -> Self {
        let function = |s: pl::Series| {
            let ca = s.str()?;
            Ok(Some(ca.str_len_chars().into_series()))
        };
        self.clone()
            .0
            .map(function, pl::GetOutput::from_type(pl::DataType::UInt32))
            .with_fmt("str.len_chars")
            .into()
    }

    pub fn str_concat(&self, delimiter: Robj, ignore_nulls: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .concat(robj_to!(str, delimiter)?, robj_to!(bool, ignore_nulls)?)
            .into())
    }

    pub fn str_to_uppercase(&self) -> Self {
        self.0.clone().str().to_uppercase().into()
    }

    pub fn str_to_lowercase(&self) -> Self {
        self.0.clone().str().to_lowercase().into()
    }

    pub fn str_to_titlecase(&self) -> RResult<Self> {
        f_str_to_titlecase(self)
    }

    pub fn str_strip_chars(&self, matches: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .strip_chars(robj_to!(PLExpr, matches)?)
            .into())
    }

    pub fn str_strip_chars_end(&self, matches: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .strip_chars_end(robj_to!(PLExpr, matches)?)
            .into())
    }

    pub fn str_strip_chars_start(&self, matches: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .strip_chars_start(robj_to!(PLExpr, matches)?)
            .into())
    }

    pub fn str_zfill(&self, alignment: Robj) -> RResult<Self> {
        Ok(self
            .clone()
            .0
            .str()
            .zfill(robj_to!(PLExprCol, alignment)?)
            .into())
    }

    pub fn str_pad_end(&self, width: Robj, fillchar: Robj) -> RResult<Self> {
        Ok(self
            .clone()
            .0
            .str()
            .pad_end(robj_to!(usize, width)?, robj_to!(char, fillchar)?)
            .into())
    }

    pub fn str_pad_start(&self, width: Robj, fillchar: Robj) -> RResult<Self> {
        Ok(self
            .clone()
            .0
            .str()
            .pad_start(robj_to!(usize, width)?, robj_to!(char, fillchar)?)
            .into())
    }

    pub fn str_contains(&self, pat: &RPolarsExpr, literal: Nullable<bool>, strict: bool) -> Self {
        RPolarsExpr(match null_to_opt(literal) {
            Some(true) => self.0.clone().str().contains_literal(pat.0.clone()),
            _ => self.0.clone().str().contains(pat.0.clone(), strict),
        })
    }

    pub fn str_ends_with(&self, sub: &RPolarsExpr) -> Self {
        self.0.clone().str().ends_with(sub.0.clone()).into()
    }

    pub fn str_starts_with(&self, sub: &RPolarsExpr) -> Self {
        self.0.clone().str().starts_with(sub.0.clone()).into()
    }

    pub fn str_json_path_match(&self, pat: Robj) -> List {
        let res = || -> Result<RPolarsExpr, String> {
            use pl::*;
            let pat: String = robj_to!(String, pat, "in str$json_path_match: {}")?;
            let function = move |s: Series| {
                let ca = s.str()?;
                match ca.json_path_match(&pat) {
                    Ok(ca) => Ok(Some(ca.into_series())),
                    Err(e) => Err(pl::PolarsError::ComputeError(format!("{e:?}").into())),
                }
            };
            Ok(RPolarsExpr(
                self.0
                    .clone()
                    .map(function, pl::GetOutput::from_type(pl::DataType::String))
                    .with_fmt("str.json_path_match"),
            ))
        }();
        r_result_list(res)
    }

    pub fn str_json_decode(&self, dtype: Robj, infer_schema_len: Robj) -> RResult<Self> {
        let dtype = robj_to!(Option, RPolarsDataType, dtype)?.map(|dty| dty.0);
        let infer_schema_len = robj_to!(Option, usize, infer_schema_len)?;
        Ok(self
            .0
            .clone()
            .str()
            .json_decode(dtype, infer_schema_len)
            .into())
    }

    pub fn str_hex_encode(&self) -> RResult<Self> {
        Ok(self.0.clone().str().hex_encode().into())
    }

    pub fn str_hex_decode(&self, strict: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .hex_decode(robj_to!(bool, strict)?)
            .into())
    }
    pub fn str_base64_encode(&self) -> RResult<Self> {
        Ok(self.0.clone().str().base64_encode().into())
    }

    pub fn str_base64_decode(&self, strict: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .base64_decode(robj_to!(bool, strict)?)
            .into())
    }

    pub fn str_extract(&self, pattern: Robj, group_index: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .extract(robj_to!(PLExprCol, pattern)?, robj_to!(usize, group_index)?)
            .into())
    }

    pub fn str_extract_all(&self, pattern: &RPolarsExpr) -> Self {
        self.0.clone().str().extract_all(pattern.0.clone()).into()
    }

    pub fn str_count_matches(&self, pattern: Robj, literal: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .count_matches(robj_to!(PLExpr, pattern)?, robj_to!(bool, literal)?)
            .into())
    }

    pub fn str_to_date(
        &self,
        format: Robj,
        strict: Robj,
        exact: Robj,
        cache: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .to_date(pl::StrptimeOptions {
                format: robj_to!(Option, String, format)?,
                strict: robj_to!(bool, strict)?,
                exact: robj_to!(bool, exact)?,
                cache: robj_to!(bool, cache)?,
            })
            .into())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn str_to_datetime(
        &self,
        format: Robj,
        time_unit: Robj,
        time_zone: Robj,
        strict: Robj,
        exact: Robj,
        cache: Robj,
        ambiguous: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .to_datetime(
                robj_to!(Option, timeunit, time_unit)?,
                robj_to!(Option, String, time_zone)?,
                pl::StrptimeOptions {
                    format: robj_to!(Option, String, format)?,
                    strict: robj_to!(bool, strict)?,
                    exact: robj_to!(bool, exact)?,
                    cache: robj_to!(bool, cache)?,
                },
                robj_to!(PLExpr, ambiguous)?,
            )
            .into())
    }

    pub fn str_to_time(&self, format: Robj, strict: Robj, cache: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .to_time(pl::StrptimeOptions {
                format: robj_to!(Option, String, format)?,
                strict: robj_to!(bool, strict)?,
                cache: robj_to!(bool, cache)?,
                exact: true,
            })
            .into())
    }

    pub fn str_split(&self, by: Robj, inclusive: Robj) -> Result<RPolarsExpr, String> {
        let by = robj_to!(PLExpr, by)?;
        let inclusive = robj_to!(bool, inclusive)?;
        if inclusive {
            Ok(self.0.clone().str().split_inclusive(by).into())
        } else {
            Ok(self.0.clone().str().split(by).into())
        }
    }

    pub fn str_split_exact(
        &self,
        by: Robj,
        n: Robj,
        inclusive: Robj,
    ) -> Result<RPolarsExpr, String> {
        let by = robj_to!(PLExpr, by)?;
        let n = robj_to!(usize, n)?;
        let inclusive = robj_to!(bool, inclusive)?;
        Ok(if inclusive {
            self.0.clone().str().split_exact_inclusive(by, n)
        } else {
            self.0.clone().str().split_exact(by, n)
        }
        .into())
    }

    pub fn str_splitn(&self, by: Robj, n: Robj) -> Result<RPolarsExpr, String> {
        Ok(self
            .0
            .clone()
            .str()
            .splitn(robj_to!(PLExpr, by)?, robj_to!(usize, n)?)
            .into())
    }

    pub fn str_replace(
        &self,
        pattern: Robj,
        value: Robj,
        literal: Robj,
    ) -> Result<RPolarsExpr, String> {
        Ok(self
            .0
            .clone()
            .str()
            .replace(
                robj_to!(Expr, pattern)?.0,
                robj_to!(Expr, value)?.0,
                robj_to!(bool, literal)?,
            )
            .into())
    }

    pub fn str_replace_all(
        &self,
        pattern: Robj,
        value: Robj,
        literal: Robj,
    ) -> Result<RPolarsExpr, String> {
        Ok(self
            .0
            .clone()
            .str()
            .replace_all(
                robj_to!(Expr, pattern)?.0,
                robj_to!(Expr, value)?.0,
                robj_to!(bool, literal)?,
            )
            .into())
    }

    pub fn str_slice(&self, offset: Robj, length: Robj) -> Result<RPolarsExpr, String> {
        let offset = robj_to!(PLExprCol, offset)?;
        let length = robj_to!(PLExprCol, length)?;

        Ok(self.clone().0.str().slice(offset, length).into())
    }

    pub fn str_explode(&self) -> Result<RPolarsExpr, String> {
        Ok(self.0.clone().str().explode().into())
    }

    pub fn str_parse_int(&self, radix: Robj, strict: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .to_integer(robj_to!(u32, radix)?, robj_to!(bool, strict)?)
            .with_fmt("str.parse_int")
            .into())
    }

    pub fn str_reverse(&self) -> RResult<Self> {
        Ok(self.0.clone().str().reverse().into())
    }

    pub fn str_contains_any(&self, patterns: Robj, ascii_case_insensitive: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .contains_any(
                robj_to!(PLExpr, patterns)?,
                robj_to!(bool, ascii_case_insensitive)?,
            )
            .into())
    }

    pub fn str_replace_many(
        &self,
        patterns: Robj,
        replace_with: Robj,
        ascii_case_insensitive: Robj,
    ) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .str()
            .replace_many(
                robj_to!(PLExpr, patterns)?,
                robj_to!(PLExpr, replace_with)?,
                robj_to!(bool, ascii_case_insensitive)?,
            )
            .into())
    }

    //binary methods
    pub fn bin_contains(&self, lit: Robj) -> RResult<Self> {
        Ok(self
            .0
            .clone()
            .binary()
            .contains_literal(robj_to!(PLExpr, lit)?)
            .into())
    }

    pub fn bin_starts_with(&self, sub: Robj) -> Result<Self, String> {
        Ok(self
            .0
            .clone()
            .binary()
            .starts_with(robj_to!(PLExpr, sub)?)
            .into())
    }

    pub fn bin_ends_with(&self, sub: Robj) -> Result<Self, String> {
        Ok(self
            .0
            .clone()
            .binary()
            .ends_with(robj_to!(PLExpr, sub)?)
            .into())
    }

    pub fn bin_encode_hex(&self) -> Self {
        self.0
            .clone()
            .map(
                move |s| s.binary().map(|s| Some(s.hex_encode().into_series())),
                pl::GetOutput::same_type(),
            )
            .with_fmt("binary.hex_encode")
            .into()
    }

    pub fn bin_encode_base64(&self) -> Self {
        self.0
            .clone()
            .map(
                move |s| s.binary().map(|s| Some(s.base64_encode().into_series())),
                pl::GetOutput::same_type(),
            )
            .with_fmt("binary.base64_encode")
            .into()
    }

    pub fn bin_decode_hex(&self, strict: bool) -> Self {
        self.0
            .clone()
            .map(
                move |s| {
                    s.binary()?
                        .hex_decode(strict)
                        .map(|s| Some(s.into_series()))
                },
                pl::GetOutput::same_type(),
            )
            .with_fmt("binary.hex_decode")
            .into()
    }

    pub fn bin_decode_base64(&self, strict: bool) -> Self {
        self.0
            .clone()
            .map(
                move |s| {
                    s.binary()?
                        .base64_decode(strict)
                        .map(|s| Some(s.into_series()))
                },
                pl::GetOutput::same_type(),
            )
            .with_fmt("binary.base64_decode")
            .into()
    }

    pub fn struct_field_by_name(&self, name: Robj) -> Result<RPolarsExpr, String> {
        Ok(self
            .0
            .clone()
            .struct_()
            .field_by_name(robj_to!(str, name)?)
            .into())
    }

    // pub fn struct_field_by_index(&self, index: i64) -> PyExpr {
    //     self.0.clone().struct_().field_by_index(index).into()
    // }

    pub fn struct_rename_fields(&self, names: Robj) -> Result<RPolarsExpr, String> {
        let string_vec: Vec<String> = robj_to!(Vec, String, names)?;
        Ok(self.0.clone().struct_().rename_fields(string_vec).into())
    }

    //placed in py-polars/src/lazy/meta.rs, however extendr do not support
    //multiple export impl.
    fn meta_pop(&self) -> List {
        let exprs: Vec<pl::Expr> = self.0.clone().meta().pop();
        List::from_values(exprs.iter().map(|e| RPolarsExpr(e.clone())))
    }

    fn meta_eq(&self, other: Robj) -> Result<bool, String> {
        let other = robj_to!(Expr, other)?;
        Ok(self.0 == other.0)
    }

    fn meta_roots(&self) -> Vec<String> {
        self.0
            .clone()
            .meta()
            .root_names()
            .iter()
            .map(|name| name.to_string())
            .collect()
    }

    fn meta_output_name(&self) -> Result<String, String> {
        let name = self
            .0
            .clone()
            .meta()
            .output_name()
            .map_err(|err| err.to_string())?;

        Ok(name.to_string())
    }

    fn meta_undo_aliases(&self) -> RPolarsExpr {
        self.0.clone().meta().undo_aliases().into()
    }

    fn meta_has_multiple_outputs(&self) -> bool {
        self.0.clone().meta().has_multiple_outputs()
    }

    fn meta_is_regex_projection(&self) -> bool {
        self.0.clone().meta().is_regex_projection()
    }

    fn meta_tree_format(&self) -> RResult<String> {
        let e = self
            .0
            .clone()
            .meta()
            .into_tree_formatter()
            .map_err(polars_to_rpolars_err)?;
        Ok(format!("{e}"))
    }

    fn cat_set_ordering(&self, ordering: Robj) -> Result<RPolarsExpr, String> {
        let ordering = robj_to!(Map, str, ordering, |s| {
            Ok(crate::rdatatype::new_categorical_ordering(s).map_err(Rctx::Plain)?)
        })?;
        Ok(self
            .0
            .clone()
            .cast(pl::DataType::Categorical(None, ordering))
            .into())
    }

    fn cat_get_categories(&self) -> RPolarsExpr {
        self.0.clone().cat().get_categories().into()
    }

    // external expression function which typically starts a new expression chain
    // to avoid name space collisions in R, these static methods are not free functions
    // as in py-polars. prefix with new_ to not collide with other methods in class
    pub fn new_len() -> RPolarsExpr {
        dsl::len().into()
    }

    pub fn new_first() -> RPolarsExpr {
        dsl::first().into()
    }

    pub fn new_last() -> RPolarsExpr {
        dsl::last().into()
    }

    pub fn cov(a: Robj, b: Robj, ddof: Robj) -> RResult<RPolarsExpr> {
        Ok(pl::cov(
            robj_to!(PLExprCol, a)?,
            robj_to!(PLExprCol, b)?,
            robj_to!(u8, ddof)?,
        )
        .into())
    }

    pub fn rolling_cov(
        a: Robj,
        b: Robj,
        window_size: Robj,
        min_periods: Robj,
        ddof: Robj,
    ) -> RResult<Self> {
        Ok(pl::rolling_cov(
            robj_to!(PLExprCol, a)?,
            robj_to!(PLExprCol, b)?,
            new_rolling_cov_options(window_size, min_periods, ddof)?,
        )
        .into())
    }

    pub fn corr(a: Robj, b: Robj, method: Robj, ddof: Robj, propagate_nans: Robj) -> RResult<Self> {
        let x = robj_to!(PLExprCol, a)?;
        let y = robj_to!(PLExprCol, b)?;
        let df = robj_to!(u8, ddof)?;
        match robj_to!(String, method)?.as_str() {
            "pearson" => Ok(pl::pearson_corr(x, y, df).into()),
            "spearman" => {
                Ok(pl::spearman_rank_corr(x, y, df, robj_to!(bool, propagate_nans)?).into())
            }
            m => rerr()
                .bad_val(m)
                .misvalued("should be 'pearson' or 'spearman'"),
        }
    }

    pub fn rolling_corr(
        a: Robj,
        b: Robj,
        window_size: Robj,
        min_periods: Robj,
        ddof: Robj,
    ) -> RResult<Self> {
        Ok(pl::rolling_corr(
            robj_to!(PLExprCol, a)?,
            robj_to!(PLExprCol, b)?,
            new_rolling_cov_options(window_size, min_periods, ddof)?,
        )
        .into())
    }

    pub fn rolling(
        &self,
        index_column: Robj,
        period: Robj,
        offset: Robj,
        closed: Robj,
        check_sorted: Robj,
    ) -> RResult<Self> {
        let index_column = robj_to!(String, index_column)?.into();
        let period = Duration::parse(robj_to!(str, period)?);
        let offset = Duration::parse(robj_to!(str, offset)?);
        let closed_window = robj_to!(ClosedWindow, closed)?;
        let check_sorted = robj_to!(bool, check_sorted)?;

        let options = RollingGroupOptions {
            index_column,
            period,
            offset,
            closed_window,
            check_sorted,
        };

        Ok(self.0.clone().rolling(options).into())
    }
}

// handle varition in implementation if not the nightly feature
// could not get cfg feature flags conditions to work inside extendr macro
// Therefore place it outside here instead
#[allow(unused)]
fn f_str_to_titlecase(expr: &RPolarsExpr) -> RResult<RPolarsExpr> {
    #[cfg(feature = "nightly")]
    return (Ok(expr.0.clone().str().to_titlecase().into()));

    #[cfg(not(feature = "nightly"))]
    rerr().plain("$to_titlecase() is only available with the 'nightly' feature")
}

//allow proto expression that yet only are strings
//string expression will transformed into an actual expression in different contexts such as select
#[derive(Clone, Debug)]
pub enum ProtoRexpr {
    RPolarsExpr(RPolarsExpr),
    String(String),
}

#[extendr]
impl ProtoRexpr {
    pub fn new_str(s: &str) -> Self {
        ProtoRexpr::String(s.to_owned())
    }

    pub fn new_expr(r: &RPolarsExpr) -> Self {
        ProtoRexpr::RPolarsExpr(r.clone())
    }

    pub fn to_rexpr(&self, context: &str) -> RPolarsExpr {
        match self {
            ProtoRexpr::RPolarsExpr(r) => r.clone(),
            ProtoRexpr::String(s) => match context {
                "select" => RPolarsExpr::col(s),
                _ => panic!("unknown context"),
            },
        }
    }

    fn print(&self) {
        rprintln!("{:#?}", self);
    }
}

//and array of expression or proto expressions.
#[derive(Clone, Debug)]
pub struct RPolarsProtoExprArray(pub Vec<ProtoRexpr>);

impl Default for RPolarsProtoExprArray {
    fn default() -> Self {
        Self::new()
    }
}

#[extendr]
impl RPolarsProtoExprArray {
    pub fn new() -> Self {
        RPolarsProtoExprArray(Vec::new())
    }

    pub fn push_back_str(&mut self, s: &str) {
        self.0.push(ProtoRexpr::new_str(s));
    }

    pub fn push_back_rexpr(&mut self, r: &RPolarsExpr) {
        self.0.push(ProtoRexpr::new_expr(r));
    }

    pub fn print(&self) {
        rprintln!("{:#?}", self);
    }
}

impl RPolarsProtoExprArray {
    pub fn to_vec(&self, context: &str) -> Vec<pl::Expr> {
        self.0.iter().map(|re| re.to_rexpr(context).0).collect()
    }
}

//external function as extendr-api do not allow methods returning unwrapped structs
//deprecate use method instead
pub fn pra_to_vec(pra: &RPolarsProtoExprArray, context: &str) -> Vec<pl::Expr> {
    pra.0.iter().map(|re| re.to_rexpr(context).0).collect()
}

//make options rolling options from R friendly arguments, handle conversion errors
pub fn make_rolling_options(
    window_size: Robj,
    weights: Robj,
    min_periods: Robj,
    center: Robj,
    by_null: Robj,
    closed_null: Robj,
    warn_if_unsorted: Robj,
) -> RResult<pl::RollingOptions> {
    Ok(pl::RollingOptions {
        window_size: pl::Duration::parse(robj_to!(str, window_size)?),
        weights: robj_to!(Option, Vec, f64, weights)?,
        min_periods: robj_to!(usize, min_periods)?,
        center: robj_to!(bool, center)?,
        by: robj_to!(Option, String, by_null)?,
        closed_window: robj_to!(Option, ClosedWindow, closed_null)?,
        warn_if_unsorted: robj_to!(bool, warn_if_unsorted)?,
        ..Default::default()
    })
}

// #[derive(Clone, Debug)]
// pub struct When {
//     predicate: Expr,
// }

// #[derive(Clone, Debug)]
// pub struct Then {
//     predicate: Expr,
//     then: Expr,
// }

// #[derive(Clone)]
// pub struct ChainWhen(dsl::ChainWhen);

// #[extendr]
// impl WhenThenThen {
//     pub fn when(&self, predicate: &RPolarsExpr) -> WhenThenThen {
//         Self(self.0.clone().when(predicate.0.clone()))
//     }
//     pub fn then(&self, expr: &RPolarsExpr) -> WhenThenThen {
//         Self(self.0.clone().then(expr.0.clone()))
//     }
//     pub fn otherwise(&self, expr: &RPolarsExpr) -> RPolarsExpr {
//         self.0.clone().otherwise(expr.0.clone()).into()
//     }

//     pub fn print(&self) {
//         rprintln!("Polars WhenThenThen");
//     }
// }

// #[derive(Clone)]
// pub struct ChainThen(dsl::ChainThen);

// #[extendr]
// impl Then {
//     pub fn when(&self, predicate: &RPolarsExpr) -> WhenThenThen {
//         let e = dsl::when(self.predicate.0.clone())
//             .then(self.then.0.clone())
//             .when(predicate.0.clone());
//         WhenThenThen(e)
//     }

//     pub fn otherwise(&self, expr: &RPolarsExpr) -> RPolarsExpr {
//         dsl::ternary_expr(
//             self.predicate.0.clone(),
//             self.then.0.clone(),
//             expr.0.clone(),
//         )
//         .into()
//     }

//     pub fn print(&self) {
//         rprintln!("{:?}", self);
//     }
// }

// #[extendr]
// impl When {
//     #[allow(clippy::self_named_constructors)]
//     pub fn when(predicate: &RPolarsExpr) -> When {
//         When {
//             predicate: predicate.clone(),
//         }
//     }

//     pub fn then(&self, expr: &RPolarsExpr) -> WhenThen {
//         WhenThen {
//             predicate: self.predicate.clone(),
//             then: expr.clone(),
//         }
//     }

//     pub fn print(&self) {
//         rprintln!("{:?}", self);
//     }
// }

#[extendr]
pub fn internal_wrap_e(robj: Robj, str_to_lit: Robj) -> RResult<RPolarsExpr> {
    if robj_to!(bool, str_to_lit)? {
        robj_to!(Expr, robj)
    } else {
        robj_to!(ExprCol, robj)
    }
}

#[extendr]
pub fn robj_to_col(name: Robj, dotdotdot: Robj) -> RResult<RPolarsExpr> {
    //let last_type_str = None;

    || -> RResult<RPolarsExpr> {
        use crate::utils::unpack_r_eval;
        let name = if name.inherits("RPolarsSeries") {
            unpack_r_eval(R!("polars:::result({{name}}$to_vector())"))?
        } else {
            name
        };

        match () {
            _ if name.is_string() && name.len() == 1 && dotdotdot.len() == 0 => {
                Ok(RPolarsExpr::col(name.as_str().unwrap_or("")))
            }
            _ if name.inherits("RPolarsDataType")
                //or if name is a list and first element is RPolarsDataType
                | if let Some(Ok(robj)) = name.as_list().map(|l| l.elt(0)) {
                    robj.inherits("RPolarsDataType")
                } else {
                    false
                } =>
            {
                let mut name = robj_to!(Vec, PLPolarsDataType, name)?;
                let mut ddd = robj_to!(Vec, PLPolarsDataType, dotdotdot)?;
                name.append(&mut ddd);
                Ok(RPolarsExpr(dsl::dtype_cols(name)))
            }

            _ => {
                let mut name = robj_to!(Vec, String, name)?;
                let mut ddd = robj_to!(Vec, String, dotdotdot)?;
                name.append(&mut ddd);

                Ok(RPolarsExpr::cols(name))
            }
        }
    }()
    .when("constructing a Column Expr")

    // if ddd.length() > 0 {}
    // if res.is_ok() {
    //     return res;
    // } else {
    //     res_dtype = robj_to!(Vec, RPolarsDataType, name);
    //     if res_dtype.is_ok() {
    //         return ();
    //     }
    // }
    // let ddd = dotdotdot.as_list().unwrap();
}

#[extendr]
extendr_module! {
    mod dsl;
    impl RPolarsExpr;
    impl RPolarsProtoExprArray;
    fn internal_wrap_e;
    fn robj_to_col;


}
