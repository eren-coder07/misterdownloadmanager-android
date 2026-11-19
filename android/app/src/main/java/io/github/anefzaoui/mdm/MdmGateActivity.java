package io.github.anefzaoui.mdm;

import android.app.Activity;
import android.app.ActivityManager;
import android.app.ApplicationExitInfo;
import android.app.NativeActivity;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.graphics.Typeface;
import android.os.Build;
import android.os.Bundle;
import android.text.TextUtils;
import android.util.Log;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import java.io.File;
import java.io.RandomAccessFile;
import java.nio.charset.StandardCharsets;
import java.util.List;

/**
 * The launcher's gate: on the way into the app it checks whether the last
 * launch ended in a crash, and when it did, this activity shows the crash
 * report on screen and copies it to the clipboard, so the reason reaches
 * someone who can fix it. A file manager cannot open the app's private folder
 * on newer Android, an app may not read the system's log at all, and a launch
 * that dies before its first frame otherwise leaves nothing behind — this is
 * where the report comes from instead:
 *
 * - the app's own log (mdm-debug.log, mirrored from logcat by the Rust side),
 *   its panics and its native-crash lines included;
 * - a Java exception the last launch wrote (the native library's "dlopen
 *   failed" among them — that one ends the app before any Rust code runs);
 * - the system's own exit reason for the process (Android 11 and later).
 *
 * Every launch without a crash passes straight through to the NativeActivity.
 */
public class MdmGateActivity extends Activity {
    /** The Rust side's marker: "starting" while it runs, "exited"/"error" when it ends by itself. */
    private static final String MARKER = "run-state";
    /** What a Java-side crash wrote last launch (the handler below). */
    private static final String JAVA_CRASH = "crash-java.txt";
    /** The app's own log (mdm-android mirrors logcat there, and truncates it at each launch). */
    private static final String LOG = "mdm-debug.log";
    /** How much of the log's end a report carries. */
    private static final long TAIL_BYTES = 64 * 1024;

    /** The system's handler is wrapped once per process. */
    private static boolean crashHandlerInstalled;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        installJavaCrashHandler();
        requestNotifications();
        String report = buildReport();
        if (report == null) {
            startMdm();
            return;
        }
        copy(report);
        setContentView(makeUi(report));
    }

    /** Hand the launch to the app itself. */
    private void startMdm() {
        startActivity(new Intent(this, NativeActivity.class));
        finish();
        overridePendingTransition(0, 0);
    }

    // ---- The report -----------------------------------------------------------------------

    /**
     * The crash report, or null when the last launch ended by itself (and no
     * Java exception is waiting): no report, straight into the app.
     */
    private String buildReport() {
        File javaCrash = new File(getFilesDir(), JAVA_CRASH);
        String javaCrashText = readText(javaCrash);
        boolean markerStarting = "starting".equals(readText(new File(getFilesDir(), MARKER)));
        String logTail = readLogTail();
        boolean logSaysCrash = logTail != null
                && (logTail.contains("NATIVE CRASH") || logTail.contains("PANIC"));
        boolean exitSaysCrash = exitWasCrash();
        String exit = exitSummary();
        boolean hasExit = !exit.isEmpty();

        // A crash: a Java exception was written, or the Rust side never got to
        // end its launch ("starting" still there) and the system's exit reason
        // says a crash. Without an exit reason (older Android), the log's own
        // panic and native-crash lines say it.
        boolean crash = javaCrashText != null
                || (markerStarting && (exitSaysCrash || (!hasExit && logSaysCrash)));
        if (!crash) {
            return null;
        }

        StringBuilder b = new StringBuilder();
        b.append("MDM crash report — ").append(versionName()).append('\n');
        b.append("device: ").append(Build.MANUFACTURER).append(' ').append(Build.MODEL)
                .append("; Android ").append(Build.VERSION.RELEASE)
                .append(" (SDK ").append(Build.VERSION.SDK_INT).append(")\n");
        b.append("ABIs: ").append(TextUtils.join(", ", Build.SUPPORTED_ABIS)).append('\n');
        b.append("the last launch ended in a crash");
        if (!exit.isEmpty()) {
            b.append("\nexit reason: ").append(exit);
        }
        if (javaCrashText != null) {
            b.append("\n\n--- Java exception (last launch) ---\n").append(javaCrashText);
            javaCrash.delete(); // shown once; a new crash writes the file again
        } else {
            b.append("\n\n(no Java exception: the crash was inside the app's native code, "
                    + "or before it started)");
        }
        b.append("\n\n--- MDM's own log (mdm-debug.log tail) ---\n");
        b.append(logTail == null
                ? "(nothing — the app died before it logged anything)"
                : logTail);
        return b.toString();
    }

    /** The system's exit reason for the process, "" where there is none (or none to tell). */
    private String exitSummary() {
        ApplicationExitInfo info = lastExit();
        if (info == null) {
            return "";
        }
        String description = info.getDescription();
        String name = exitName(info.getReason());
        return description == null || description.isEmpty()
                ? name
                : name + " — " + description;
    }

    private boolean exitWasCrash() {
        ApplicationExitInfo info = lastExit();
        if (info == null) {
            return false;
        }
        int reason = info.getReason();
        return reason == ApplicationExitInfo.REASON_CRASH
                || reason == ApplicationExitInfo.REASON_CRASH_NATIVE
                || reason == ApplicationExitInfo.REASON_ANR
                || reason == ApplicationExitInfo.REASON_LOW_MEMORY
                || reason == ApplicationExitInfo.REASON_SIGNALED
                || reason == ApplicationExitInfo.REASON_EXCESSIVE_RESOURCE_USAGE;
    }

    /** The most recent exit record, or null (older Android, or none kept). */
    private ApplicationExitInfo lastExit() {
        if (Build.VERSION.SDK_INT < 30) {
            return null;
        }
        try {
            ActivityManager am = getSystemService(ActivityManager.class);
            List<ApplicationExitInfo> list =
                    am.getHistoricalProcessExitReasons(getPackageName(), 0, 5);
            return list.isEmpty() ? null : list.get(0);
        } catch (Throwable t) {
            return null;
        }
    }

    private static String exitName(int reason) {
        switch (reason) {
            case ApplicationExitInfo.REASON_CRASH: return "Java crash";
            case ApplicationExitInfo.REASON_CRASH_NATIVE: return "native crash (a signal)";
            case ApplicationExitInfo.REASON_ANR: return "not responding (ANR: the window froze)";
            case ApplicationExitInfo.REASON_LOW_MEMORY: return "killed: the device was out of memory";
            case ApplicationExitInfo.REASON_SIGNALED: return "killed by a signal";
            case ApplicationExitInfo.REASON_USER_REQUESTED: return "user requested (swiped away)";
            case ApplicationExitInfo.REASON_USER_STOPPED: return "user stopped";
            case ApplicationExitInfo.REASON_EXIT_SELF: return "exited by itself";
            case ApplicationExitInfo.REASON_OTHER: return "other";
            default: return "reason " + reason;
        }
    }

    // ---- The Java side's crashes ----------------------------------------------------------

    /**
     * Java exceptions (the library load's "dlopen failed" among them — that
     * one happens before any Rust code runs) land in a file the next launch's
     * report reads. The system's own handling carries on: its dialog still
     * shows, and Android still records the exit.
     */
    private void installJavaCrashHandler() {
        if (crashHandlerInstalled) {
            return;
        }
        crashHandlerInstalled = true;
        final Thread.UncaughtExceptionHandler system = Thread.getDefaultUncaughtExceptionHandler();
        Thread.setDefaultUncaughtExceptionHandler((t, e) -> {
            try {
                String logTail = readLogTail(getFilesDir());
                writeText(new File(getFilesDir(), JAVA_CRASH),
                        "Java exception on thread " + (t == null ? "?" : t.getName()) + ": " + e
                                + "\n" + Log.getStackTraceString(e)
                                + "\n\n--- MDM's own log tail ---\n"
                                + (logTail == null ? "(none)" : logTail));
            } catch (Throwable ignored) {
                // A dying process writes what it can.
            }
            if (system != null) {
                system.uncaughtException(t, e);
            }
        });
    }

    /** The app may show reminders (the Rust side posts one on a failed start): ask once. */
    private void requestNotifications() {
        if (Build.VERSION.SDK_INT >= 33
                && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
                        != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[]{android.Manifest.permission.POST_NOTIFICATIONS}, 1);
        }
    }

    // ---- The screen -----------------------------------------------------------------------

    private View makeUi(String report) {
        float density = getResources().getDisplayMetrics().density;
        int pad = (int) (16 * density);

        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setBackgroundColor(Color.BLACK);
        root.setPadding(pad, pad * 2, pad, pad);

        TextView title = new TextView(this);
        title.setText("MDM did not finish its last launch");
        title.setTextSize(18);
        title.setTypeface(Typeface.DEFAULT_BOLD);
        title.setTextColor(Color.WHITE);
        root.addView(title);

        TextView hint = new TextView(this);
        hint.setText("The report is copied to the clipboard already. Copy or share it below, "
                + "then start MDM.");
        hint.setTextSize(13);
        hint.setTextColor(0xFFB0B0B0);
        hint.setPadding(0, (int) (6 * density), 0, 0);
        root.addView(hint);

        ScrollView scroll = new ScrollView(this);
        TextView text = new TextView(this);
        text.setText(report);
        text.setTextSize(11);
        text.setTypeface(Typeface.MONOSPACE);
        text.setTextColor(0xFFE0E0E0);
        text.setTextIsSelectable(true);
        scroll.addView(text);
        root.addView(scroll, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));

        LinearLayout row = new LinearLayout(this);
        row.setOrientation(LinearLayout.HORIZONTAL);
        row.setGravity(Gravity.CENTER);
        LinearLayout.LayoutParams rowSpec = new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT);
        rowSpec.setMargins(0, (int) (10 * density), 0, 0);

        Button copy = new Button(this);
        copy.setText("Copy");
        copy.setOnClickListener(v -> copy(report));
        row.addView(copy, new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1f));

        Button share = new Button(this);
        share.setText("Share");
        share.setOnClickListener(v -> share(report));
        row.addView(share, new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        root.addView(row, rowSpec);

        Button start = new Button(this);
        start.setText("Start MDM");
        start.setOnClickListener(v -> startMdm());
        root.addView(start, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        return root;
    }

    private void copy(String report) {
        try {
            ClipboardManager cm = getSystemService(ClipboardManager.class);
            cm.setPrimaryClip(ClipData.newPlainText("MDM crash report", report));
            Toast.makeText(this, "Crash report copied to the clipboard", Toast.LENGTH_SHORT).show();
        } catch (Throwable t) {
            Toast.makeText(this, "Copy failed — use Share instead", Toast.LENGTH_SHORT).show();
        }
    }

    private void share(String report) {
        Intent send = new Intent(Intent.ACTION_SEND);
        send.setType("text/plain");
        send.putExtra(Intent.EXTRA_SUBJECT, "MDM crash report");
        send.putExtra(Intent.EXTRA_TEXT, report);
        startActivity(Intent.createChooser(send, "Send the crash report"));
    }

    // ---- Small file readers and writers ---------------------------------------------------

    private static String readText(File file) {
        if (!file.isFile()) {
            return null;
        }
        try {
            byte[] bytes = new byte[(int) Math.min(file.length(), TAIL_BYTES)];
            try (java.io.FileInputStream in = new java.io.FileInputStream(file)) {
                int read = in.read(bytes);
                return new String(bytes, 0, Math.max(read, 0), StandardCharsets.UTF_8);
            }
        } catch (Throwable t) {
            return null;
        }
    }

    private static void writeText(File file, String text) {
        try (java.io.FileOutputStream out = new java.io.FileOutputStream(file)) {
            out.write(text.getBytes(StandardCharsets.UTF_8));
        } catch (Throwable t) {
            // A dying process writes what it can.
        }
    }

    /** The log's last lines (the log is this launch's own: the Rust side truncates it). */
    private static String readLogTail(File filesDir) {
        File file = new File(filesDir, LOG);
        if (!file.isFile()) {
            return null;
        }
        try (RandomAccessFile raf = new RandomAccessFile(file, "r")) {
            long len = raf.length();
            long start = Math.max(0, len - TAIL_BYTES);
            raf.seek(start);
            byte[] bytes = new byte[(int) (len - start)];
            raf.readFully(bytes);
            String text = new String(bytes, StandardCharsets.UTF_8);
            if (start > 0) {
                int newline = text.indexOf('\n');
                if (newline >= 0) {
                    text = text.substring(newline + 1);
                }
            }
            return text;
        } catch (Throwable t) {
            return null;
        }
    }

    private String readLogTail() {
        return readLogTail(getFilesDir());
    }

    private String versionName() {
        try {
            return getPackageManager().getPackageInfo(getPackageName(), 0).versionName;
        } catch (Throwable t) {
            return "?";
        }
    }
}
