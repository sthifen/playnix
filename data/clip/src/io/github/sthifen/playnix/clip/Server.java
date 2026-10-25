package io.github.sthifen.playnix.clip;

import android.content.ClipData;
import android.content.ClipDescription;
import android.net.Uri;
import android.os.Binder;
import android.os.IBinder;
import android.os.Parcel;

import java.io.BufferedOutputStream;
import java.io.ByteArrayOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.FileDescriptor;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Portapapeles de Android para `playnix clipboard`. Corre como shell con
 * `adb shell CLASSPATH=<este apk> app_process / io.github.sthifen.playnix.clip.Server`
 * y usa la API interna (IClipboard), como scrcpy: shell puede leerlo en segundo plano.
 *
 * Por stdin y stdout van mensajes [tipo u8][largo u32][datos]: 0 = texto UTF-8,
 * 1 = imagen (de Linux siempre PNG; de Android PNG, JPEG o WebP). Al arrancar escribe
 * MAGIC para que Linux sepa dónde empiezan los mensajes. Termina cuando se cierra stdin.
 *
 * Las imágenes viajan como URI content://. Las de Android se leen con `content read`
 * (al pedir el portapapeles, Android le concede a shell permiso sobre sus URI). Las de
 * Linux se guardan en ClipProvider (solo shell puede escribir en él) y se pone su URI.
 *
 * Si el portapapeles propio de Waydroid está activo (lo registra su hwcomposer),
 * Android devuelve siempre el texto de Linux al leer el portapapeles y las imágenes no
 * llegan a las apps; en ese caso solo se pasa texto. Playnix lo desactiva con
 * libpnxh.so (data/hwc-fix), si la versión de Waydroid es compatible.
 */
public final class Server {
    private static final byte[] MAGIC = "PNXCLIP1".getBytes(StandardCharsets.US_ASCII);
    private static final int TEXT = 0;
    private static final int IMAGE = 1;
    private static final int MAX = 32 << 20;
    private static final String PACKAGE = "com.android.shell";
    private static final String PROVIDER = "content://io.github.sthifen.playnix.clip/";

    private static Object service;
    private static Method get, set, addListener;
    /** URI que pusimos nosotros (para no devolverle a Linux su propia imagen). */
    private static String ownUri;
    private static int serial;
    /** El portapapeles del host de Waydroid está activo (ver arriba). */
    private static boolean hostClipboard;
    private static DataOutputStream out;
    /** Lecturas y escrituras del portapapeles en orden, fuera del hilo de binder. */
    private static final ExecutorService worker = Executors.newSingleThreadExecutor();

    public static void main(String[] args) throws Exception {
        out = new DataOutputStream(new BufferedOutputStream(new FileOutputStream(FileDescriptor.out)));
        connect();
        ClipData now = (ClipData) get.invoke(service, pkgArgs(get.getParameterCount()));
        hostClipboard = now != null && now.getDescription() != null
            && "host clipboard".equals(String.valueOf(now.getDescription().getLabel()));
        if (hostClipboard) System.err.println("playnix-clip: portapapeles de Waydroid activo, solo texto");
        synchronized (Server.class) {
            out.write(MAGIC);
            out.flush();
        }
        listen(() -> worker.execute(Server::changed));

        DataInputStream in = new DataInputStream(new FileInputStream(FileDescriptor.in));
        try {
            while (true) {
                int kind = in.readUnsignedByte();
                int len = in.readInt();
                if (len < 0 || len > MAX) break;
                byte[] data = new byte[len];
                in.readFully(data);
                worker.execute(() -> {
                    try {
                        if (kind == IMAGE) writeImage(data);
                        else if (kind == TEXT) writeText(new String(data, StandardCharsets.UTF_8));
                    } catch (Exception e) {
                        log(e);
                    }
                });
            }
        } catch (EOFException e) {
            // Linux cerró la conexión
        }
        System.exit(0);
    }

    private static void changed() {
        try {
            ClipData clip = (ClipData) get.invoke(service, pkgArgs(get.getParameterCount()));
            if (clip == null || clip.getItemCount() == 0) return;
            ClipData.Item item = clip.getItemAt(0);
            Uri uri = item.getUri();
            ClipDescription desc = clip.getDescription();
            if (uri != null && desc != null && desc.getMimeTypeCount() > 0 && desc.getMimeType(0).startsWith("image/")) {
                if (uri.toString().equals(ownUri)) return;
                byte[] data = readUri(uri.toString());
                if (data != null) send(IMAGE, data);
                return;
            }
            CharSequence text = item.getText();
            if (text == null && item.getHtmlText() != null) text = item.getHtmlText();
            if (text != null) send(TEXT, text.toString().getBytes(StandardCharsets.UTF_8));
        } catch (Exception e) {
            log(e);
        }
    }

    private static void log(Throwable e) {
        if (e instanceof java.lang.reflect.InvocationTargetException && e.getCause() != null) e = e.getCause();
        System.err.println("playnix-clip: " + e);
    }

    private static synchronized void send(int kind, byte[] data) throws Exception {
        out.writeByte(kind);
        out.writeInt(data.length);
        out.write(data);
        out.flush();
    }

    private static void connect() throws Exception {
        IBinder b = (IBinder) Class.forName("android.os.ServiceManager")
            .getMethod("getService", String.class).invoke(null, "clipboard");
        service = Class.forName("android.content.IClipboard$Stub")
            .getMethod("asInterface", IBinder.class).invoke(null, b);
        for (Method m : service.getClass().getMethods()) {
            switch (m.getName()) {
                case "getPrimaryClip": get = m; break;
                case "setPrimaryClip": set = m; break;
                case "addPrimaryClipChangedListener": addListener = m; break;
                default:
            }
        }
        if (get == null || set == null || addListener == null) throw new Exception("IClipboard incompleto");
    }

    /**
     * Paquete y usuario según la versión de Android:
     * (pkg) · (pkg, user) · (pkg, tag, user) · (pkg, tag, user, device).
     */
    private static Object[] pkgArgs(int count) {
        Object[] a = new Object[count];
        a[0] = PACKAGE;
        if (count == 2) a[1] = 0;
        if (count >= 3) a[2] = 0;
        if (count >= 4) a[3] = 0;
        return a;
    }

    /** Un primer argumento y después paquete y usuario. */
    private static Object[] args(Object first, int count) {
        Object[] a = new Object[count];
        a[0] = first;
        System.arraycopy(pkgArgs(count - 1), 0, a, 1, count - 1);
        return a;
    }

    private static void listen(Runnable listener) throws Exception {
        // IOnPrimaryClipChangedListener es oneway y tiene un solo método:
        // basta un Binder que responda a cualquier transacción.
        Binder binder = new Binder() {
            @Override
            protected boolean onTransact(int code, Parcel data, Parcel reply, int flags) {
                if (code >= FIRST_CALL_TRANSACTION && code <= LAST_CALL_TRANSACTION) {
                    listener.run();
                    return true;
                }
                return false;
            }
        };
        Object proxy = Class.forName("android.content.IOnPrimaryClipChangedListener$Stub")
            .getMethod("asInterface", IBinder.class).invoke(null, binder);
        addListener.invoke(service, args(proxy, addListener.getParameterCount()));
    }

    private static void writeText(String text) throws Exception {
        ClipData clip = ClipData.newPlainText("Playnix", text);
        set.invoke(service, args(clip, set.getParameterCount()));
    }

    /** Guarda la imagen en ClipProvider y pone su URI. */
    private static void writeImage(byte[] png) throws Exception {
        // Waydroid pondría el texto vacío del ítem en el portapapeles de Linux
        if (hostClipboard) return;
        String uri = PROVIDER + "clip-" + (++serial) + "-" + System.currentTimeMillis() + ".png";
        Process p = Runtime.getRuntime().exec(new String[] {"content", "write", "--uri", uri});
        try (OutputStream o = p.getOutputStream()) {
            o.write(png);
        }
        String err = drain(p.getErrorStream()) + drain(p.getInputStream());
        if (p.waitFor() != 0 || err.contains("Exception")) throw new Exception("no se pudo guardar la imagen: " + err.trim());
        ClipDescription desc = new ClipDescription("Playnix", new String[] {"image/png"});
        // Con texto vacío: Waydroid parchea ClipboardService y llama getText().toString()
        ClipData clip = new ClipData(desc, new ClipData.Item("", null, Uri.parse(uri)));
        ownUri = uri;
        set.invoke(service, args(clip, set.getParameterCount()));
    }

    private static byte[] readUri(String uri) throws Exception {
        Process p = Runtime.getRuntime().exec(new String[] {"content", "read", "--uri", uri});
        p.getOutputStream().close();
        ByteArrayOutputStream buf = new ByteArrayOutputStream();
        try (InputStream in = p.getInputStream()) {
            byte[] chunk = new byte[65536];
            int n;
            while ((n = in.read(chunk)) > 0) {
                buf.write(chunk, 0, n);
                if (buf.size() > MAX) {
                    p.destroy();
                    return null;
                }
            }
        }
        p.waitFor();
        byte[] d = buf.toByteArray();
        // `content read` escribe los errores en stdout en algunas versiones
        boolean png = d.length > 8 && (d[0] & 0xFF) == 0x89 && d[1] == 'P';
        boolean jpeg = d.length > 3 && (d[0] & 0xFF) == 0xFF && (d[1] & 0xFF) == 0xD8;
        boolean webp = d.length > 12 && d[8] == 'W' && d[9] == 'E';
        return png || jpeg || webp ? d : null;
    }

    private static String drain(InputStream in) throws Exception {
        ByteArrayOutputStream buf = new ByteArrayOutputStream();
        byte[] chunk = new byte[4096];
        int n;
        while ((n = in.read(chunk)) > 0) buf.write(chunk, 0, n);
        return buf.toString("UTF-8");
    }
}
