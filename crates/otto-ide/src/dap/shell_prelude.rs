//! Out-of-band `bash` (`trap ... DEBUG`) and `zsh` (`TRAPDEBUG` + `funcfiletrace`)
//! debugger prelude scripts for [`super::shell_bridge`].

#[cfg(unix)]
pub(super) const BASH_PRELUDE: &str = r#"
_otto_fifo_evt="$OTTO_SH_DAP_EVT"
_otto_fifo_cmd="$OTTO_SH_DAP_CMD"
_otto_bps_file="$OTTO_SH_DAP_BPS"
_otto_pause_file="$OTTO_SH_DAP_PAUSE"
_otto_target="$OTTO_SH_DAP_TARGET"
unset OTTO_SH_DAP_EVT OTTO_SH_DAP_CMD OTTO_SH_DAP_BPS OTTO_SH_DAP_PAUSE OTTO_SH_DAP_TARGET

exec 8<"$_otto_fifo_cmd"
exec 9>"$_otto_fifo_evt"
printf 'READY\n' >&9

_otto_mode="step_in"
_otto_target_depth=0
_otto_bps="|"
_otto_active=0

while IFS= read -r _otto_init_cmd <&8; do
  case "$_otto_init_cmd" in
    BPS\|*) _otto_bps="${_otto_init_cmd#BPS}" ;;
    RUN\|*) _otto_mode="${_otto_init_cmd#RUN|}"; break ;;
    QUIT) exit 0 ;;
  esac
done

_otto_trap_handler() {
  [ "$_otto_active" -eq 1 ] || return 0
  [ "${BASH_SUBSHELL:-0}" -eq 0 ] || return 0
  local _fn="${FUNCNAME[1]:-main}"
  case "$_fn" in
    _otto_*) return 0 ;;
  esac
  local _src="${BASH_SOURCE[1]:-}"
  local _ln="${1:-1}"
  [ -z "$_src" ] && return 0
  case "$_src" in
    *prelude.bash) return 0 ;;
    /*) ;;
    *) _src="$PWD/$_src" ;;
  esac

  if [ -r "$_otto_bps_file" ]; then
    IFS= read -r _otto_bps < "$_otto_bps_file" || true
  fi
  local _paused_req=0
  if [ -e "$_otto_pause_file" ]; then
    rm -f "$_otto_pause_file" 2>/dev/null || true
    _paused_req=1
  fi

  local _depth="${#FUNCNAME[@]}"
  local _hit=0
  case "$_otto_bps" in
    *"|$_src:$_ln|"*) _hit=1 ;;
  esac

  local _stop=0
  if [ "$_hit" -eq 1 ] || [ "$_paused_req" -eq 1 ]; then
    _stop=1
  elif [ "$_otto_mode" = "step_in" ]; then
    _stop=1
  elif [ "$_otto_mode" = "step_over" ] && [ "$_depth" -le "$_otto_target_depth" ]; then
    _stop=1
  elif [ "$_otto_mode" = "step_out" ] && [ "$_depth" -lt "$_otto_target_depth" ]; then
    _stop=1
  fi
  [ "$_stop" -eq 1 ] || return 0

  local _reason="step"
  [ "$_paused_req" -eq 1 ] && _reason="pause"
  [ "$_hit" -eq 1 ] && _reason="breakpoint"

  printf 'STOP|%s|%s|%s|%s\n' "$_reason" "$_fn" "$_src" "$_ln" >&9
  local _i=1
  local _max="${#BASH_SOURCE[@]}"
  while [ "$_i" -lt "$_max" ]; do
    local _f_name="${FUNCNAME[$_i]:-main}"
    local _f_src="${BASH_SOURCE[$_i]:-$_src}"
    local _f_ln="$_ln"
    if [ "$_i" -gt 1 ]; then
      local _prev=$((_i - 1))
      _f_ln="${BASH_LINENO[$_prev]:-1}"
    fi
    printf 'FRAME|%s|%s|%s\n' "$_f_name" "$_f_src" "$_f_ln" >&9
    _i=$((_i + 1))
  done

  while IFS='=' read -r _vk _vv; do
    case "$_vk" in
      _otto_*|BASH*|FUNCNAME|LINENO|OPTIND|OPTERR|PPID|SHELLOPTS|UID|EUID|GROUPS|IFS|_*|*[!A-Za-z0-9_]*) ;;
      *) printf 'VAR|%s|%s\n' "$_vk" "${_vv:0:512}" >&9 ;;
    esac
  done <<EOF_VARS
$(set -o posix; set)
EOF_VARS
  printf 'END_STOP\n' >&9

  while IFS= read -r _cmd <&8; do
    case "$_cmd" in
      BPS\|*) _otto_bps="${_cmd#BPS}" ;;
      CONT) _otto_mode="run"; return 0 ;;
      NEXT) _otto_mode="step_over"; _otto_target_depth="$_depth"; return 0 ;;
      STEP) _otto_mode="step_in"; return 0 ;;
      OUT)  _otto_mode="step_out"; _otto_target_depth="$_depth"; return 0 ;;
      EVAL\|*)
        local _expr="${_cmd#EVAL|}"
        local _out=""
        local _rc=0
        _out="$(eval "$_expr" 2>&1)" || _rc=$?
        _out="${_out//$'\n'/ }"
        printf 'EVAL_RES|%s|%s\n' "$_rc" "$_out" >&9
        ;;
      QUIT) exit 0 ;;
    esac
  done
}

trap '_otto_active=0; trap - DEBUG 2>/dev/null || true; printf "EXIT\n" >&9 2>/dev/null || true' EXIT
set -T
_otto_active=1
trap '_otto_trap_handler "$LINENO"' DEBUG
. "$_otto_target" "$@"
_otto_active=0
trap - DEBUG
printf 'EXIT\n' >&9 2>/dev/null || true
"#;

#[cfg(unix)]
pub(super) const ZSH_PRELUDE: &str = r#"
emulate -LR zsh
setopt DEBUG_BEFORE_CMD
zmodload zsh/parameter 2>/dev/null || true

_otto_fifo_evt="$OTTO_SH_DAP_EVT"
_otto_fifo_cmd="$OTTO_SH_DAP_CMD"
_otto_bps_file="$OTTO_SH_DAP_BPS"
_otto_pause_file="$OTTO_SH_DAP_PAUSE"
_otto_target="$OTTO_SH_DAP_TARGET"
unset OTTO_SH_DAP_EVT OTTO_SH_DAP_CMD OTTO_SH_DAP_BPS OTTO_SH_DAP_PAUSE OTTO_SH_DAP_TARGET

exec 8<"$_otto_fifo_cmd"
exec 9>"$_otto_fifo_evt"
printf 'READY\n' >&9

_otto_mode="step_in"
_otto_target_depth=0
_otto_bps="|"
_otto_active=0

while IFS= read -r _otto_init_cmd <&8; do
  case "$_otto_init_cmd" in
    BPS\|*) _otto_bps="${_otto_init_cmd#BPS}" ;;
    RUN\|*) _otto_mode="${_otto_init_cmd#RUN|}"; break ;;
    QUIT) exit 0 ;;
  esac
done

TRAPDEBUG() {
  [[ "$_otto_active" -eq 1 ]] || return 0
  [[ "${ZSH_SUBSHELL:-0}" -eq 0 ]] || return 0
  local _loc="${funcfiletrace[1]:-}"
  [[ -n "$_loc" ]] || return 0
  local _src="${_loc%:*}"
  local _ln="${_loc##*:}"
  [[ "$_src" == *"prelude.zsh" ]] && return 0
  case "$_src" in
    /*) ;;
    *) _src="$PWD/$_src" ;;
  esac
  local _fn="${funcstack[2]:-main}"
  [[ "$_fn" == "$_otto_target" ]] && _fn="main"

  if [[ -r "$_otto_bps_file" ]]; then
    IFS= read -r _otto_bps < "$_otto_bps_file" || true
  fi
  local _paused_req=0
  if [[ -e "$_otto_pause_file" ]]; then
    rm -f "$_otto_pause_file" 2>/dev/null || true
    _paused_req=1
  fi

  local _depth="${#funcfiletrace[@]}"
  local _hit=0
  case "$_otto_bps" in
    *"|$_src:$_ln|"*) _hit=1 ;;
  esac

  local _stop=0
  if [[ "$_hit" -eq 1 || "$_paused_req" -eq 1 ]]; then
    _stop=1
  elif [[ "$_otto_mode" == "step_in" ]]; then
    _stop=1
  elif [[ "$_otto_mode" == "step_over" && "$_depth" -le "$_otto_target_depth" ]]; then
    _stop=1
  elif [[ "$_otto_mode" == "step_out" && "$_depth" -lt "$_otto_target_depth" ]]; then
    _stop=1
  fi
  [[ "$_stop" -eq 1 ]] || return 0

  local _reason="step"
  [[ "$_paused_req" -eq 1 ]] && _reason="pause"
  [[ "$_hit" -eq 1 ]] && _reason="breakpoint"

  printf 'STOP|%s|%s|%s|%s\n' "$_reason" "$_fn" "$_src" "$_ln" >&9
  local _idx=1
  local _max="${#funcfiletrace[@]}"
  while [[ "$_idx" -le "$_max" ]]; do
    local _floc="${funcfiletrace[$_idx]}"
    local _fsrc="${_floc%:*}"
    local _fln="${_floc##*:}"
    [[ "$_fsrc" == *"prelude.zsh" ]] && break
    local _sidx=$((_idx + 1))
    local _fname="${funcstack[$_sidx]:-main}"
    [[ "$_fname" == "$_otto_target" ]] && _fname="main"
    printf 'FRAME|%s|%s|%s\n' "$_fname" "$_fsrc" "$_fln" >&9
    _idx=$((_idx + 1))
  done

  local _k _v _pt
  for _k in ${(k)parameters}; do
    _pt="${parameters[$_k]}"
    [[ "$_pt" == *special* ]] && continue
    case "$_k" in
      _otto_*|ZSH*|func*|LINENO|OPTIND|OPTERR|PPID|UID|EUID|EGID|GID|IFS|PATH|FPATH|MANPATH|signals|_*|*[!A-Za-z0-9_]*) ;;
      *)
        _v="${(P)_k}"
        _v="${_v//$'\n'/ }"
        printf 'VAR|%s|%s\n' "$_k" "${_v[1,512]}" >&9
        ;;
    esac
  done
  printf 'END_STOP\n' >&9

  local _cmd
  while IFS= read -r _cmd <&8; do
    case "$_cmd" in
      BPS\|*) _otto_bps="${_cmd#BPS}" ;;
      CONT) _otto_mode="run"; return 0 ;;
      NEXT) _otto_mode="step_over"; _otto_target_depth="$_depth"; return 0 ;;
      STEP) _otto_mode="step_in"; return 0 ;;
      OUT)  _otto_mode="step_out"; _otto_target_depth="$_depth"; return 0 ;;
      EVAL\|*)
        local _expr="${_cmd#EVAL|}"
        local _out=""
        local _rc=0
        _out="$(eval "$_expr" 2>&1)" || _rc=$?
        _out="${_out//$'\n'/ }"
        printf 'EVAL_RES|%s|%s\n' "$_rc" "$_out" >&9
        ;;
      QUIT) exit 0 ;;
    esac
  done
}

TRAPEXIT() {
  _otto_active=0
  printf 'EXIT\n' >&9 2>/dev/null || true
}
_otto_active=1
source "$_otto_target" "$@"
_otto_active=0
printf 'EXIT\n' >&9 2>/dev/null || true
"#;
