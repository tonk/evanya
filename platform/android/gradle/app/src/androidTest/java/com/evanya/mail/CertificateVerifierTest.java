package com.evanya.mail;

import static org.junit.Assert.assertNotEquals;

import android.content.Context;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import org.junit.Test;
import org.junit.runner.RunWith;

/** Check that the release dependency includes the JVM API called through Rust JNI. */
@RunWith(AndroidJUnit4.class)
public final class CertificateVerifierTest {
    @Test
    public void packagedVerifierRejectsMalformedCertificate() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        Class<?> verifier = context.getClassLoader()
                .loadClass("org.rustls.platformverifier.CertificateVerifier");
        Method verify = verifier.getDeclaredMethod("verifyCertificateChain", Context.class,
                String.class, String.class, String[].class, byte[].class, long.class, byte[][].class);
        verify.setAccessible(true);
        Object result = verify.invoke(null, context, "example.test", "RSA", new String[0],
                new byte[0], System.currentTimeMillis() / 1000, new byte[][]{new byte[]{0}});
        Field code = result.getClass().getDeclaredField("code");
        code.setAccessible(true);
        assertNotEquals("Malformed certificates must fail verification", 0, code.getInt(result));
    }
}
