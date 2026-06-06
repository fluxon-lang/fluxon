#!/usr/bin/env bash
# Flux .fx test to'plamini ishga tushiruvchi. Toza master ustida mavjud
# imkoniyatlarni (yadro til, kolleksiyalar+modullar, db, time+env, http) sinaydi.
#
# Ishga (lokal):  ./tests-fx/run_all.sh        # runtime/ papkasidan
# Ishga (CI):     FLUX_BIN=target/release/flux ./tests-fx/run_all.sh
#
# FLUX_BIN — flux binary'ga yo'l (standart: ./target/release/flux). DIR esa shu
# skript joylashgan papkadan aniqlanadi, qaysi cwd'dan chaqirilsa ham ishlaydi.
set -u
BIN="${FLUX_BIN:-./target/release/flux}"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
pass=0; fail=0

run() {  # run <label> <env...> -- <fx-fayl>
  local label="$1"; shift
  local out
  out="$("$@" 2>&1)"
  echo "$out"
  if echo "$out" | grep -q "HAMMASI O'TDI"; then
    echo ">>> $label: PASS"; pass=$((pass+1))
  else
    echo ">>> $label: FAIL"; fail=$((fail+1))
  fi
  echo
}

echo "############ 01 yadro til ############"
run "01_core"               $BIN run $DIR/01_core.fx

echo "############ 02 kolleksiyalar + modullar ############"
run "02_collections_modules" $BIN run $DIR/02_collections_modules.fx

echo "############ 03 db (in-memory SQLite) ############"
run "03_db"                 env DATABASE_URL=sqlite::memory: $BIN run $DIR/03_db.fx

echo "############ 03b db.tx rollback ############"
DB="/tmp/flux_rb_$$.db"; rm -f "$DB"
env DATABASE_URL="sqlite:$DB" $BIN run $DIR/03b_db_rollback.fx >/dev/null 2>&1
rb_out="$(env DATABASE_URL="sqlite:$DB" $BIN run $DIR/03b_check.fx 2>&1)"
echo "$rb_out"
if echo "$rb_out" | grep -q "ROLLBACK-OK"; then
  echo ">>> 03b_rollback: PASS"; pass=$((pass+1))
else
  echo ">>> 03b_rollback: FAIL"; fail=$((fail+1))
fi
rm -f "$DB"
echo

echo "############ 04 time + env ############"
run "04_time_env"  env FLUX_TEST_VAR=salom PORT=9090 $BIN run $DIR/04_time_env.fx

echo "############ 05 http (server + klient) ############"
$BIN run $DIR/05_http_server.fx >/tmp/flux_srv_$$.log 2>&1 &
SRV=$!
for i in $(seq 1 40); do
  curl -s -o /dev/null http://127.0.0.1:8123/health 2>/dev/null && break
  perl -e 'select(undef,undef,undef,0.2)'
done
http_out="$($BIN run $DIR/05_http_client.fx 2>&1)"
echo "$http_out"
kill "$SRV" 2>/dev/null; wait "$SRV" 2>/dev/null
rm -f /tmp/flux_srv_$$.log
if echo "$http_out" | grep -q "HAMMASI O'TDI"; then
  echo ">>> 05_http: PASS"; pass=$((pass+1))
else
  echo ">>> 05_http: FAIL"; fail=$((fail+1))
fi
echo

echo "############ 06 reg (funksiya registri) ############"
run "06_reg"                $BIN run $DIR/06_reg.fx

echo "############ 07 cron (rejalashtirilgan vazifalar) ############"
run "07_cron"               $BIN run $DIR/07_cron.fx

echo "############ 08 io (terminal input/output) ############"
# io stdin'dan o'qiydi — standart run() stdin bermaydi, shuning uchun quvuraymiz.
io_out="$(printf 'Firdavs\n42\n' | $BIN run $DIR/08_io.fx 2>&1)"
echo "$io_out"
if echo "$io_out" | grep -q "HAMMASI O'TDI"; then
  echo ">>> 08_io: PASS"; pass=$((pass+1))
else
  echo ">>> 08_io: FAIL"; fail=$((fail+1))
fi
echo

echo "############ 09 fs (lokal fayl tizimi) ############"
run "09_fs"                 $BIN run $DIR/09_fs.fx

echo "############ 10 ai (LLM primitiv — tarmoqsiz) ############"
# ai.ask/json/run haqiqiy chaqiruvi $AI_KEY + tarmoq talab qiladi; bu test faqat
# shadowing va tool-loop'ning Flux tomonini sinaydi (token sarflamaydi).
run "10_ai"                 $BIN run $DIR/10_ai.fx

echo "================================================"
echo "YAKUN: $pass PASS, $fail FAIL"
echo "================================================"
[ "$fail" -eq 0 ]
