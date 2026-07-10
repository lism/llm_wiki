"""SSH to server - check Oracle SID=110 direct path write."""
import subprocess, os, tempfile

HOST = "199.66.62.239"
USER = "root"
PASS = "admin@123"

askpass = tempfile.NamedTemporaryFile(mode="w", suffix=".sh", delete=False)
askpass.write("#!/bin/sh\necho '" + PASS + "'\n")
askpass.close()
os.chmod(askpass.name, 0o700)

cmds = """
echo '=== SID=110 CURRENT SQL ==='
su - oracle -c 'sqlplus -s / as sysdba << EOF
set lines 300 pages 50
SELECT s.sid, s.serial#, s.status, s.event, s.seconds_in_wait, s.last_call_et, s.sql_id, s.prev_sql_id, s.osuser, s.machine, s.program, s.module, s.action
FROM v\\$session s WHERE s.sid = 110;
EOF'

echo ''
echo '=== SQL TEXT (current) ==='
su - oracle -c 'sqlplus -s / as sysdba << EOF
set lines 300 pages 50
SELECT sql_id, sql_fulltext FROM v\\$sql WHERE sql_id = (SELECT sql_id FROM v\\$session WHERE sid = 110);
EOF'

echo ''
echo '=== SQL TEXT (prev) ==='
su - oracle -c 'sqlplus -s / as sysdba << EOF
set lines 300 pages 50
SELECT sql_id, sql_fulltext FROM v\\$sql WHERE sql_id = (SELECT prev_sql_id FROM v\\$session WHERE sid = 110) AND ROWNUM = 1;
EOF'

echo ''
echo '=== OPEN CURSORS for SID=110 ==='
su - oracle -c 'sqlplus -s / as sysdba << EOF
set lines 300 pages 50
SELECT sql_id, sql_text FROM v\\$open_cursor WHERE sid = 110 AND ROWNUM <= 5;
EOF'

echo ''
echo '=== SESSION I/O STATS ==='
su - oracle -c 'sqlplus -s / as sysdba << EOF
set lines 300
SELECT s.sid, n.name, s.value FROM v\\$statname n, v\\$sesstat s
WHERE n.statistic# = s.statistic# AND s.sid = 110
AND n.name IN ('"'"'physical writes direct'"'"', '"'"'physical writes direct temporary'"'"', '"'"'physical reads direct'"'"', '"'"'physical writes'"'"', '"'"'physical reads'"'"', '"'"'redo size'"'"');
EOF'

echo ''
echo '=== ALL DIRECT PATH WRITE SESSIONS ==='
su - oracle -c 'sqlplus -s / as sysdba << EOF
set lines 200
SELECT s.sid, s.serial#, s.sql_id, s.event, s.seconds_in_wait, s.osuser, s.program
FROM v\\$session s WHERE s.event = '"'"'direct path write'"'"';
EOF'
"""

p = subprocess.Popen(
    ["ssh", "-o", "StrictHostKeyChecking=no", f"{USER}@{HOST}", cmds],
    stdin=subprocess.DEVNULL,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    env={**os.environ, "SSH_ASKPASS": askpass.name, "DISPLAY": "dummy", "SSH_ASKPASS_REQUIRE": "force"},
)
out, err = p.communicate(timeout=30)
os.unlink(askpass.name)

if out:
    print(out.decode())
if err:
    print("STDERR:", err.decode())
