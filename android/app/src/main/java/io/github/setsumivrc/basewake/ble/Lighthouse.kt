package io.github.setsumivrc.basewake.ble

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothProfile
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.os.Build
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import java.io.IOException
import java.util.UUID

/** Power characteristic values of a Valve Lighthouse V2 base station. */
object Power {
    const val SLEEP = 0x00
    const val ON = 0x01
    const val STANDBY = 0x02
}

/** A base station heard during a scan. */
data class Found(val mac: String, val name: String)

class LighthouseException(message: String) : IOException(message)

/**
 * Talks to Lighthouse V2 base stations over BLE. Every operation opens its own GATT connection
 * and closes it when done, so stations can be controlled in parallel.
 *
 * Callers must hold the Bluetooth permissions before calling in.
 */
@SuppressLint("MissingPermission")
class Lighthouse(private val context: Context, private val adapter: BluetoothAdapter) {

    /** Listens for base stations until the flow is cancelled. Emits each one every time it is heard. */
    fun scan(): Flow<Found> = callbackFlow {
        val scanner = adapter.bluetoothLeScanner ?: throw LighthouseException("Bluetooth is off")
        val callback = object : ScanCallback() {
            override fun onScanResult(callbackType: Int, result: ScanResult) {
                val name = result.scanRecord?.deviceName ?: runCatching { result.device.name }.getOrNull()
                if (name != null && name.startsWith(NAME_PREFIX)) {
                    trySend(Found(result.device.address, name))
                }
            }

            override fun onBatchScanResults(results: MutableList<ScanResult>) {
                results.forEach { onScanResult(0, it) }
            }

            override fun onScanFailed(errorCode: Int) {
                close(LighthouseException("Scan failed (error $errorCode)"))
            }
        }
        val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
        scanner.startScan(null, settings, callback)
        awaitClose { runCatching { scanner.stopScan(callback) } }
    }

    /** Reads the power state byte. */
    suspend fun read(mac: String): Int = withContext(Dispatchers.IO) {
        retrying(mac) { client -> client.read() }
    }

    /**
     * Writes [target] and reads it back to confirm, retrying up to [ATTEMPTS] times.
     * Returns the state read back, which for [Power.ON] may still be a startup value.
     */
    suspend fun write(mac: String, target: Int): Int = withContext(Dispatchers.IO) {
        retrying(mac) { client ->
            client.write(target)
            delay(300)
            val state = client.read()
            if (!confirms(target, state)) {
                throw LighthouseException("Station reported 0x%02X after writing 0x%02X".format(state, target))
            }
            state
        }
    }

    /** Runs [op] on a connected station, reconnecting and trying again on failure. */
    private suspend fun <T> retrying(mac: String, op: suspend (GattClient) -> T): T {
        var last: Exception? = null
        repeat(ATTEMPTS) { attempt ->
            if (attempt > 0) delay(500L * attempt)
            val client = GattClient(context, adapter.getRemoteDevice(mac))
            try {
                withTimeout(CONNECT_TIMEOUT_MS) { client.connect() }
                return withTimeout(OP_TIMEOUT_MS) { op(client) }
            } catch (e: TimeoutCancellationException) {
                last = LighthouseException("Timed out")
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                last = e
            } finally {
                client.close()
            }
        }
        throw last ?: LighthouseException("Unreachable")
    }

    companion object {
        const val NAME_PREFIX = "LHB-"
        val SERVICE: UUID = UUID.fromString("00001523-1212-efde-1523-785feabcd124")
        val POWER: UUID = UUID.fromString("00001525-1212-efde-1523-785feabcd124")
        private const val ATTEMPTS = 3
        private const val CONNECT_TIMEOUT_MS = 10_000L
        private const val OP_TIMEOUT_MS = 5_000L

        /** Whether [state], read after writing [target], shows the write took. */
        fun confirms(target: Int, state: Int): Boolean = when (target) {
            // Waking up goes through startup values before settling on 0x01.
            Power.ON -> state != Power.SLEEP && state != Power.STANDBY
            else -> state == target
        }
    }
}

/** One GATT connection to one station, with the callbacks turned into suspending calls. */
@SuppressLint("MissingPermission")
private class GattClient(private val context: Context, private val device: BluetoothDevice) : BluetoothGattCallback() {
    private var gatt: BluetoothGatt? = null
    private var power: BluetoothGattCharacteristic? = null
    private val connected = CompletableDeferred<Unit>()
    private val discovered = CompletableDeferred<Unit>()

    @Volatile
    private var pendingRead: CompletableDeferred<ByteArray>? = null

    @Volatile
    private var pendingWrite: CompletableDeferred<Unit>? = null

    suspend fun connect() {
        gatt = device.connectGatt(context, false, this, BluetoothDevice.TRANSPORT_LE)
            ?: throw LighthouseException("Could not connect")
        connected.await()
        if (gatt?.discoverServices() != true) throw LighthouseException("Could not discover services")
        discovered.await()
        power = gatt?.getService(Lighthouse.SERVICE)?.getCharacteristic(Lighthouse.POWER)
            ?: throw LighthouseException("Not a Lighthouse V2 base station")
    }

    suspend fun read(): Int {
        val g = gatt ?: throw LighthouseException("Not connected")
        val ch = power ?: throw LighthouseException("Not connected")
        val pending = CompletableDeferred<ByteArray>()
        pendingRead = pending
        if (!g.readCharacteristic(ch)) throw LighthouseException("Read failed")
        val value = pending.await()
        if (value.isEmpty()) throw LighthouseException("Empty read")
        return value[0].toInt() and 0xFF
    }

    suspend fun write(value: Int) {
        val g = gatt ?: throw LighthouseException("Not connected")
        val ch = power ?: throw LighthouseException("Not connected")
        val bytes = byteArrayOf(value.toByte())
        val pending = CompletableDeferred<Unit>()
        pendingWrite = pending
        val started = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            g.writeCharacteristic(ch, bytes, BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT) == BluetoothGatt.GATT_SUCCESS
        } else {
            @Suppress("DEPRECATION")
            ch.writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT
            @Suppress("DEPRECATION")
            ch.value = bytes
            @Suppress("DEPRECATION")
            g.writeCharacteristic(ch)
        }
        if (!started) throw LighthouseException("Write failed")
        pending.await()
    }

    fun close() {
        val g = gatt ?: return
        gatt = null
        runCatching { g.disconnect() }
        runCatching { g.close() }
    }

    private fun fail(reason: String) {
        val e = LighthouseException(reason)
        connected.completeExceptionally(e)
        discovered.completeExceptionally(e)
        pendingRead?.completeExceptionally(e)
        pendingWrite?.completeExceptionally(e)
    }

    override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
        if (status == BluetoothGatt.GATT_SUCCESS && newState == BluetoothProfile.STATE_CONNECTED) {
            connected.complete(Unit)
        } else if (newState == BluetoothProfile.STATE_DISCONNECTED || status != BluetoothGatt.GATT_SUCCESS) {
            fail("Disconnected (status $status)")
        }
    }

    override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
        if (status == BluetoothGatt.GATT_SUCCESS) discovered.complete(Unit) else fail("Service discovery failed ($status)")
    }

    override fun onCharacteristicRead(
        gatt: BluetoothGatt,
        characteristic: BluetoothGattCharacteristic,
        value: ByteArray,
        status: Int,
    ) {
        val pending = pendingRead ?: return
        if (status == BluetoothGatt.GATT_SUCCESS) pending.complete(value) else pending.completeExceptionally(
            LighthouseException("Read failed ($status)"),
        )
    }

    @Deprecated("Used before Android 13")
    @Suppress("DEPRECATION")
    override fun onCharacteristicRead(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) return
        onCharacteristicRead(gatt, characteristic, characteristic.value ?: ByteArray(0), status)
    }

    override fun onCharacteristicWrite(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic, status: Int) {
        val pending = pendingWrite ?: return
        if (status == BluetoothGatt.GATT_SUCCESS) pending.complete(Unit) else pending.completeExceptionally(
            LighthouseException("Write failed ($status)"),
        )
    }
}
