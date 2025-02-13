#' Set some options
#'
#' This is done here because it is the first file run by devtools::load_all()
#' (cf field 'Collate' in DESCRIPTION) and options are used in other internal
#' functions so options have to be defined first.
#'
#' NOTE 1: I don't think values here actually matter because this is just used
#' to build the package. Those values won't be exported in the user environment.
#' Default options are specified in .onLoad().
#'
#' NOTE 2: options on "rpool" are not defined here because they require
#' functions defined later in the process to be run. Also, they are not used in
#' other internal functions.
#'
#' @noRd
options(
  polars.debug_polars = FALSE,
  polars.df_knitr_print = "auto",
  polars.do_not_repeat_call = FALSE,
  polars.int64_conversion = "double",
  polars.limit_max_threads = NULL,
  polars.maintain_order = FALSE,
  polars.no_messages = FALSE,
  polars.strictly_immutable = TRUE
)


#' check_no_missing_args
#' @description lifecycle: DEPRECATE
#' @param fun target function to check incoming arguments for
#' @param args list of args to check
#' @param warn bool if TRUE throw warning when check fails
#' @noRd
#' @return true if args are correct
check_no_missing_args = function(
    fun, args, warn = TRUE) {
  expected_args = names(formals(fun))
  missing_args = expected_args[!expected_args %in% names(args)]
  if (length(missing_args)) {
    if (warn) {
      warning(paste(
        "Internally following arguments are not exposed:",
        paste(missing_args, collapse = ", ")
      ))
    }
    return(FALSE)
  }
  return(TRUE)
}





#' Verify user selected method/attribute exists
#' @description internal function to check method call of env_classes
#'
#' @param Class_env env_class object (the classes created by extendr-wrappers.R)
#' @param Method_name name of method requested by user
#' @param call context to throw user error, just use default
#' @param class_name NULLs
#' @noRd
#' @return invisible(NULL)
verify_method_call = function(Class_env, Method_name, call = sys.call(sys.nframe() - 2L), class_name = NULL) {
  if (polars_options()$debug_polars) {
    class_name = class_name %||% as.character(as.list(match.call())$Class_env)
    cat("[", format(subtimer_ms(), digits = 4), "ms]\n", class_name, "$", Method_name, "() -> ", sep = "")
  }
  if (!Method_name %in% names(Class_env)) {
    class_name = class_name %||% as.character(as.list(match.call())$Class_env)
    Err_plain(
      paste(
        "$ - syntax error:", Method_name, "is not a method/attribute of the class", class_name
      )
    ) |> unwrap(call = call)
  }
  invisible(NULL)
}

#' Simple SQL CASE WHEN implementation for R
#' @noRd
#' @description Inspired by data.table::fcase + dplyr::case_when.
#' Used instead of base::switch internally.
#'
#' @param ... odd arguments are bool statements, a next even argument is returned
#' if prior bool statement is the first true
#' @param or_else return this if no bool statements were true
#'
#' @details Lifecycle: perhaps replace with something written in rust to speed up a bit
#'
#' @return any return given first true bool statement otherwise value of or_else
#' @examples
#' n = 7
#' .pr$env$pcase(
#'   n < 5, "nope",
#'   n > 6, "yeah",
#'   or_else = stop("failed to have a case for n = ", n)
#' )
pcase = function(..., or_else = NULL) {
  # get unevaluated args except header-function-name and or_else
  l = head(tail(as.list(sys.call()), -1L), -1L)
  # evaluate the odd args, if TRUE, evaluate and return the next even arg
  for (i in seq_len(length(l) / 2L)) {
    if (isTRUE(eval(l[[i * 2L - 1L]], envir = parent.frame()))) {
      return(eval(l[[i * 2L]], envir = parent.frame()))
    }
  }
  or_else
}


#' Move environment elements from one env to another
#' @noRd
#' @param from_env env from
#' @param element_names names of elements to move, if named names, then name of name is to_env name
#' @param remove bool, actually remove element in from_env
#' @param to_env env to
#' @return invisble NULL
#'
move_env_elements = function(from_env, to_env, element_names, remove = TRUE) {
  names_from = element_names
  names_to = if (is.null(names(element_names))) {
    # no names defined use same name in from and to
    names_from
  } else {
    # one or more names defined, use if named else use names_from
    ifelse(nchar(names(element_names)) > 0L, names(element_names), names_from)
  }

  for (i in seq_along(element_names)) {
    name_to = names_to[i]
    name_from = names_from[i]
    to_env[[name_to]] = from_env[[name_from]]
    if (remove) rm(list = name_from, envir = from_env)
  }

  invisible(NULL)
}



#' DataFrame-list to rust vector of DataFrame
#' @description lifecycle: DEPRECATE, imple on rust side as a function
#' @param l list of DataFrame
#' @noRd
#' @return RPolarsVecDataFrame
l_to_vdf = function(l) {
  if (!length(l)) stop("cannot concat empty list l")
  do_inherit_DataFrame = sapply(l, inherits, "RPolarsDataFrame")
  if (!all(do_inherit_DataFrame)) {
    stop(paste(
      "element no(s) of concat param l:",
      paste(
        which(!do_inherit_DataFrame),
        collapse = ", "
      ),
      "are not polars DataFrame(s)"
    ))
  }

  vdf = .pr$VecDataFrame$with_capacity(length(l))
  errors = NULL
  for (item in l) {
    tryCatch(vdf$push(item), error = function(e) {
      errors <<- as.character(e)
    })
    if (!is.null(errors)) stop(errors)
  }
  vdf
}


#' Clone env on level deep.
#' @details Sometimes used in polars to produce different hashmaps(environments) containing
#' some of the same, but not all elements.
#'
#' environments are used for collections of methods and types. This function can be used to make
#' a parallel collection pointing to some of the same types. Simply copying an environment, does
#' apparently not spawn a new hashmap, and therefore the collections stay identical.
#'
#' @param env an R environment.
#' @return shallow clone of R environment
#' @noRd
#' @examples
#'
#' fruit_env = new.env(parent = emptyenv())
#' fruit_env$banana = TRUE
#' fruit_env$apple = FALSE
#'
#' env_1 = new.env(parent = emptyenv())
#' env_1$fruit_env = fruit_env
#'
#' env_naive_copy = env_1
#' env_shallow_clone = .pr$env$clone_env_one_level_deep(env_1)
#'
#' # modifying env_!
#' env_1$minerals = new.env(parent = emptyenv())
#' env_1$fruit_env$apple = 42L
#'
#' # naive copy is fully identical to env_1, so copying it not much useful
#' ls(env_naive_copy)
#' # shallow copy env does not have minerals
#' ls(env_shallow_clone)
#'
#' # however shallow clone does subscribe to changes to fruits as they were there
#' # at time of cloning
#' env_shallow_clone$fruit_env$apple
clone_env_one_level_deep = function(env) {
  newenv = new.env(parent = env)
  for (i in ls(envir = env)) newenv[[i]] = env[[i]]
  newenv
}

#' replace private class-methods with public
#' @description  extendr places the naked internal calls to rust in env-classes. This function
#' can be used to delete them and replaces them with the public methods. Which are any function
#' matching pattern typically '^CLASSNAME' e.g. '^DataFrame_' or '^Series_'. Likely only used in
#' zzz.R
#' @param env class environment to modify. Envs are mutable so no return needed
#' @param class_pattern a regex string matching declared public functions of that class
#' @param keep list of unmentioned methods to keep in public api
#' @param remove_f bool if true, will move methods, not copy
#' @noRd
#' @return side effects only
replace_private_with_pub_methods = function(env, class_pattern, keep = c(), remove_f = FALSE) {
  if (build_debug_print) cat("\n\n setting public methods for ", class_pattern)

  # get these
  class_methods = ls(parent.frame(), pattern = class_pattern)
  names(class_methods) = sub(class_pattern, "", class_methods)
  # name_methods_DataFrame = sub(class_pattern, "", class_methods)

  # any string-flag signals use internal extendr implementation directly
  use_internal_bools = sapply(class_methods, function(method) {
    x = get(method)

    isTRUE(all.equal(x, use_extendr_wrapper))
  })

  # keep internals flagged with "use_internal_method"
  null_keepers = names(class_methods)[use_internal_bools]
  if (build_debug_print) cat("\n reuse internal method :\n", paste(null_keepers, collapse = ", "))

  # remove any internal method from class, not to keep
  remove_these = setdiff(ls(env), c(keep, null_keepers))
  rm(list = remove_these, envir = env)
  if (build_debug_print) {
    cat(
      "\nInternal methods not in use or replaced :\n",
      paste(setdiff(remove_these, names(class_methods)), collapse = ", ")
    )
  }

  # write any all class methods, where not using internal directly
  if (build_debug_print) cat("\n insert derived methods:\n")
  for (i in which(!use_internal_bools)) {
    method = class_methods[i]
    if (build_debug_print) cat(method, ", ")
    env[[names(method)]] = get(method)
  }

  if (remove_f) {
    rm(list = class_methods, envir = parent.frame())
  }


  invisible(NULL)
}

## TODO deprecate and handle on rust side only
#' construct data type vector
#' @description lifecycle: Deprecate, move to rust side
#' @param l list of Expr or string
#' @return extptr to rust vector of RPolarsDataType's
#' @noRd
construct_DataTypeVector = function(l) {
  dtv = RPolarsDataTypeVector$new()
  for (i in seq_along(l)) {
    if (inherits(l[[i]], "RPolarsDataType")) {
      dtv$push(names(l)[i], l[[i]])
      next
    }
    stop(paste("element:", i, "is not a DateType"))
  }
  dtv
}


# completion symbols to method/property names.
# Can be altered to "" at session time to support e.g. vscode autocomplete which will literally
# evaluate these symbols and cause error and abort.
completion_symbols = new.env(parent = emptyenv())
completion_symbols$method = "()"
completion_symbols$setter = "<-"

#' Generate autocompletion suggestions for object
#'
#' @param env environment to extract usages from
#' @param pattern string passed to ls(pattern) to subset methods by pattern
#' @details used internally for auto completion in .DollarNames methods
#' @return method usages
#' @noRd
#' @examples
#' .pr$env$get_method_usages(.pr$env$DataFrame, pattern = "col")
get_method_usages = function(env, pattern = "") {
  found_names = ls(env, pattern = pattern)
  objects = mget(found_names, envir = env)

  facts = list(
    is_property = sapply(objects, \(x) inherits(x, "property")),
    is_setter = sapply(objects, \(x) inherits(x, "setter")),
    is_method = sapply(objects, \(x)  !inherits(x, "property") & is.function(x))
  )

  paste0_len = function(..., collapse = NULL, sep = "") {
    dot_args = list2(...)
    # any has zero length, return zero length
    if (any(!sapply(dot_args, length))) {
      character()
    } else {
      paste(..., collapse = collapse, sep = sep)
    }
  }

  if (length(facts$is_property) > 0) {
    suggestions = sort(c(
      found_names[facts$is_property],
      paste0_len(found_names[facts$is_setter], completion_symbols$setter),
      paste0_len(found_names[facts$is_method], completion_symbols$method)
    ))
  } else {
    suggestions = NULL
  }

  suggestions
}

#' Print recursively an environment, used in some documentation
#' @noRd
#' @return Print recursively an environment to the console
#' @param api env
#' @param name  name of env
#' @param max_depth numeric/int max levels to recursive iterate through
print_env = function(api, name, max_depth = 10) {
  indent_count = 1
  indentation = paste0(rep("  ", indent_count))
  show_api = function(value, name) {
    if (is.environment(value) || is.list(value)) {
      cat("\n\n", indentation, name, "(", class(value), "):")
      indent_count <<- indent_count + 1
      indentation <<- paste0(rep("  ", indent_count))
      if (indent_count > max_depth) {
        cat("\n", indentation, "not exploring deeper, increase max_depth or some circular reference?")
        return()
      }

      for (name in ls(value)) {
        if (name == "env") next()
        show_api(get(name, envir = as.environment(value)), name)
      }
      cat("\n")
      indent_count <<- indent_count - 1
      indentation <<- paste0(rep("  ", indent_count))
    } else {
      cat("\n", indentation, "[", name, ";", class(value), "]")
    }
  }
  show_api(api, name)
}



#' Reverts wrapping in I
#' @param X any Robj wrapped in `I()``
#' @details
#' https://stackoverflow.com/questions/12865218/getting-rid-of-asis-class-attribute
#' @noRd
#' @return X without any AsIs subclass
unAsIs = function(X) {
  if ("AsIs" %in% class(X)) {
    class(X) = class(X)[-match("AsIs", class(X))]
  }
  X
}

## TODO deprecate
#' restruct list
#' @description lifecycle:: Deprecate
#' Restruct an object where structs where previously unnested
#' @details
#' It was much easier impl export unnested struct from polars. This function
#' restructs exported unnested structs.
#' This function should be replaced with rust code writing this output
#' directly before nesting.
#' This hack relies on rust uses the tag "is_struct" to mark what should be re-structed.
#' @noRd
#' @param l list
#' @return restructed list
restruct_list = function(l) {
  # recursive find all struct tags in list
  explore_structs = function(x, coords) {
    if (is.list(x)) {
      if (isTRUE(attr(x, "is_struct"))) {
        structs_found_list <<- c(structs_found_list, list(coords))
      }
      for (i in seq_along(x)) explore_structs(x[[i]], coords = c(coords, i))
    }
  }
  structs_found_list = list()
  explore_structs(l, integer())

  # sort structs from deepest to shallowest
  if (!length(structs_found_list)) {
    return(l)
  }
  structs_found_list = structs_found_list |> (\(x) x[order(-sapply(x, length))])()

  val = NULL # to satisyfy R CMD check no undefined global
  # restruct all tags in list
  for (x in structs_found_list) {
    l_access_str = paste0("l", paste0("[[", x, "]]", collapse = ""))
    get_code_tokens = paste0("val <- ", l_access_str)
    eval(parse(text = get_code_tokens))
    listmapply = function(...) mapply(FUN = list, ..., SIMPLIFY = FALSE)
    new_val = do.call(listmapply, val)
    set_code_tokens = paste0(l_access_str, " <- new_val")
    eval(parse(text = set_code_tokens))
  }

  l
}


#' Macro - New subnamespace
#'
#' @description Bundle class methods into an environment (subname space)
#'
#' @param class_pattern regex to select functions
#' @param subclass_env  optional subclass of
#' @param remove_f drop sourced functions from package ns after bundling into sub ns
#' @return
#' A function which returns a subclass environment of bundled class functions.
#'
#' @details
#' This function is used to emulate py-polars subnamespace-methods
#' All R functions coined 'macro_'-functions use eval(parse()) but only at package build time
#' to solve some tricky self-referential problem. If possible to deprecate a macro in a clean way
#' , go ahead.
#' @noRd
#' @examples
#'
#' # macro_new_subnamespace() is not exported, export for this toy example
#' # macro_new_subnamespace = .pr$env$macro_new_subnamespace
#'
#' ## define some new methods prefixed 'MyClass_'
#' # MyClass_add2 = function() self + 2
#' # MyClass_mul2 = function() self * 2
#'
#' ## grab any sourced function prefixed 'MyClass_'
#' # my_class_sub_ns = macro_new_subnamespace("^MyClass_", "myclass_sub_ns")
#'
#' # here adding sub-namespace as a expr-class property/method during session-time,
#' # which only is for this demo.
#' # instead sourced method like Expr_arr() at package build time instead
#' # env = .pr$env$Expr #get env of the Expr Class
#' # env$my_sub_ns = method_as_active_binding(function() { #add a property/method
#' # my_class_sub_ns(self)
#' # })
#' # rm(env) #optional clean up
#'
#' # add user defined S3 method the subclass 'myclass_sub_ns'
#' # print.myclass_sub_ns = function(x, ...) { #add ... even if not used
#' #   print("hello world, I'm myclass_sub_ns")
#' #   print("methods in sub namespace are:")
#' #  print(ls(x))
#' #  }
#'
#' # test
#' # e = pl$lit(1:5)  #make an Expr
#' # print(e$my_sub_ns) #inspect
#' # e$my_sub_ns$add2() #use the sub namespace
#' # e$my_sub_ns$mul2()
#'
macro_new_subnamespace = function(class_pattern, subclass_env = NULL) {
  # get all methods within class
  class_methods = ls(parent.frame(), pattern = class_pattern)
  names(class_methods) = sub(class_pattern, "", class_methods)

  string = paste(
    sep = "\n",
    "function(self) {",
    "  env = new.env()",
    paste(collapse = "\n", sapply(seq_along(class_methods), function(i) {
      f_name = class_methods[i]
      m_name = names(class_methods)[i]
      f = get(f_name)
      paste0("  env$", m_name, " = ", paste(capture.output(dput(f)), collapse = "\n"))
    })),
    "  class(env) = c(subclass_env, 'method_environment',class(env))",
    "attr(env,'self') = self",
    "  env",
    "}"
  )

  if (build_debug_print) cat("new subnamespace: ", class_pattern, "\n", string)
  eval(parse(text = string))
}

#' Simple viewer of an R object based on str()
#'
#' @param x object to view.
#' @param collapse word to glue possible multilines with
#' @return string
#' @noRd
#' @examples
#' str_string(list(a = 42, c(1, 2, 3, NA)))
str_string = function(x, collapse = " ") {
  paste(capture.output(str(x)), collapse = collapse)
}




#' Verify correct time zone
#'
#' @param tz time zone string or NULL
#' @param allow_null bool, if TRUE accept NULL
#'
#' @return a result object, with either a valid string or an Err
#'
#' @noRd
#' @examples
#' check_tz_to_result = .pr$env$check_tz_to_result # expose internal
#' # return Ok
#' check_tz_to_result("GMT")
#' check_tz_to_result(NULL)
#'
#' # return Err
#' check_tz_to_result("Alice")
#' check_tz_to_result(42)
#' check_tz_to_result(NULL, allow_null = FALSE)
check_tz_to_result = function(tz, allow_null = TRUE) {
  pcase(
    is.null(tz) && !allow_null, Err_plain("pre-check tz: here NULL tz is not allowed"),
    !is.null(tz) && (!is_string(tz) || !tz %in% base::OlsonNames()), Err_plain(
      "pre-check tz: '",
      tz,
      "' is not a valid time zone string from base::OlsonNames() or NULL"
    ),
    or_else = Ok(tz)
  )
}


# this function is used in zzz.R to defined how to access methods of a subname space
sub_name_space_accessor_function = function(self, name) {
  verify_method_call(self, name, class_name = class(self)[1L])
  func = self[[name]]
  func
}

# takes a list of dtypes (for example from $schema), returns a named vector
# indicating which are Structs
dtypes_are_struct = function(dtypes) {
  sapply(dtypes, \(dt) pl$same_outer_dt(dt, pl$Struct()))
}

make_profile_plot = function(data, truncate_nodes) {
  if (!requireNamespace("ggplot2", quietly = TRUE)) {
    stop('The package "ggplot2" is required.')
  }
  timings = data$profile$to_data_frame()
  timings$node = factor(timings$node, levels = unique(timings$node))
  total_timing = max(timings$end)
  if (total_timing > 10000000) {
    unit = "s"
    total_timing = paste0(total_timing / 1000000, "s")
    timings$start = timings$start / 1000000
    timings$end = timings$end / 1000000
  } else if (total_timing > 10000) {
    unit = "ms"
    total_timing = paste0(total_timing / 1000, "ms")
    timings$start = timings$start / 1000
    timings$end = timings$end / 1000
  } else {
    unit = "\U00B5s"
    total_timing = paste0(total_timing, "\U00B5s")
  }

  # for some reason, there's an error if I use rlang::.data directly in aes()
  .data = rlang::.data

  plot = ggplot2::ggplot(
    timings,
    ggplot2::aes(
      x = .data[["start"]], xend = .data[["end"]],
      y = .data[["node"]], yend = .data[["node"]]
    )
  ) +
    ggplot2::geom_segment(linewidth = 6) +
    ggplot2::xlab(
      paste0("Node duration in ", unit, ". Total duration: ", total_timing)
    ) +
    ggplot2::ylab(NULL) +
    ggplot2::theme(
      axis.text = ggplot2::element_text(size = 12)
    )

  if (truncate_nodes > 0) {
    plot = plot +
      ggplot2::scale_y_discrete(
        labels = rev(paste0(strtrim(timings$node, truncate_nodes), "...")),
        limits = rev
      )
  } else {
    plot = plot +
      ggplot2::scale_y_discrete(
        limits = rev
      )
  }

  # do not show the plot if we're running testthat
  if (!identical(Sys.getenv("TESTTHAT"), "true")) {
    print(plot)
  }
  plot
}


# Copied from the tibble package
# https://github.com/tidyverse/tibble/blob/e78ea46caea5e89cbffa5887c11050335ab23896/R/rownames.R#L116-L118
raw_rownames = function(x) {
  .row_names_info(x, 0L) %||% .set_row_names(.row_names_info(x, 2L))
}

# from rstudioapi::isAvailable()
is_rstudio = function() {
  identical(.Platform$GUI, "RStudio")
}
