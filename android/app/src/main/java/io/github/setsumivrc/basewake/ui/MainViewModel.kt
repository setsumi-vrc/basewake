package io.github.setsumivrc.basewake.ui

import android.app.Application
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.location.LocationManager
import android.os.Build
import android.os.SystemClock
import androidx.core.content.ContextCompat
import androidx.core.location.LocationManagerCompat
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import io.github.setsumivrc.basewake.ble.Found
import io.github.setsumivrc.basewake.ble.Lighthouse
import io.github.setsumivrc.basewake.ble.Power
import io.github.setsumivrc.basewake.data.OffMode
import io.github.setsumivrc.basewake.data.Station
import io.github.setsumivrc.basewake.data.Store
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

enum class PowerState(val label: String) {
    On("On"),
    Starting("Starting"),
    Sleep("Sleep"),
    Standby("Standby"),
    Unreachable("Unreachable"),
    ;

    val powered: Boolean get() = this == On || this == Starting

    companion object {
        fun of(byte: Int): PowerState = when (byte) {
            Power.ON -> On
            Power.SLEEP -> Sleep
            Power.STANDBY -> Standby
            else -> Starting
        }
    }
}

/** Why the app can't talk to base stations right now, if it can't. */
enum class Readiness { Ready, NoBluetooth, NoPermission, BluetoothOff }

data class StationUi(
    val station: Station,
    /** Last state read, or null while it hasn't been read yet. */
    val power: PowerState?,
    /** The switch position just asked for, until the station confirms it. */
    val pending: Boolean?,
) {
    val switchOn: Boolean get() = pending ?: (power?.powered == true)
}

data class UiState(
    val stations: List<StationUi> = emptyList(),
    val offMode: OffMode = OffMode.Sleep,
    val scanning: Boolean = false,
    val readiness: Readiness = Readiness.Ready,
    val masterPending: Boolean? = null,
) {
    private val managed get() = stations.filter { it.station.managed }
    val total: Int get() = managed.size
    val on: Int get() = managed.count { it.power?.powered == true }
    val masterOn: Boolean
        get() = masterPending?.takeIf { managed.any { it.pending != null } } ?: (total > 0 && on == total)
}

private data class Live(val power: PowerState? = null, val pending: Boolean? = null)

class MainViewModel(app: Application) : AndroidViewModel(app) {
    private val store = Store(app)
    private val adapter: BluetoothAdapter? = app.getSystemService(BluetoothManager::class.java)?.adapter
    private val lighthouse = adapter?.let { Lighthouse(app, it) }

    private val live = MutableStateFlow<Map<String, Live>>(emptyMap())
    private val scanning = MutableStateFlow(false)
    private val masterPending = MutableStateFlow<Boolean?>(null)
    private val permitted = MutableStateFlow(false)
    private val bluetoothOn = MutableStateFlow(adapter?.isEnabled == true)

    /** The running command or poll per station. Only touched on the main thread. */
    private val jobs = mutableMapOf<String, Job>()
    private var lastRefresh = 0L

    private val _messages = MutableSharedFlow<String>(extraBufferCapacity = 8)
    val messages: SharedFlow<String> = _messages

    private val readiness = combine(permitted, bluetoothOn) { permitted, on ->
        when {
            adapter == null -> Readiness.NoBluetooth
            !permitted -> Readiness.NoPermission
            !on -> Readiness.BluetoothOff
            else -> Readiness.Ready
        }
    }

    val state: StateFlow<UiState> = combine(
        combine(store.stations, live) { stations, live ->
            stations.map { s -> live[s.mac].let { StationUi(s, it?.power, it?.pending) } }
        },
        store.offMode,
        scanning,
        readiness,
        masterPending,
    ) { stations, offMode, scanning, readiness, masterPending ->
        UiState(stations, offMode, scanning, readiness, masterPending)
    }.stateIn(viewModelScope, SharingStarted.Eagerly, UiState())

    private val ready get() = lighthouse != null && permitted.value && bluetoothOn.value

    private val bluetoothReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val on = intent.getIntExtra(BluetoothAdapter.EXTRA_STATE, BluetoothAdapter.ERROR) == BluetoothAdapter.STATE_ON
            bluetoothOn.value = on
            if (on) refreshAll(force = true)
        }
    }

    init {
        ContextCompat.registerReceiver(
            app,
            bluetoothReceiver,
            IntentFilter(BluetoothAdapter.ACTION_STATE_CHANGED),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
    }

    override fun onCleared() {
        runCatching { getApplication<Application>().unregisterReceiver(bluetoothReceiver) }
    }

    /** Called when the screen comes back and after a permission request. */
    fun onResume(hasPermissions: Boolean) {
        permitted.value = hasPermissions
        bluetoothOn.value = adapter?.isEnabled == true
        refreshAll(force = false)
    }

    /** Reads the state of every remembered station that isn't busy with a command. */
    fun refreshAll(force: Boolean) {
        if (!ready) return
        val now = SystemClock.elapsedRealtime()
        if (!force && now - lastRefresh < 5_000) return
        lastRefresh = now
        viewModelScope.launch {
            store.stations.first().forEach { refresh(it.mac) }
        }
    }

    private fun refresh(mac: String) {
        val lh = lighthouse ?: return
        if (jobs[mac]?.isActive == true) return
        track(mac) {
            val power = try {
                PowerState.of(lh.read(mac))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                PowerState.Unreachable
            }
            setLive(mac) { it.copy(power = power) }
            if (power == PowerState.Starting) pollUntilOn(mac)
        }
    }

    fun setPower(mac: String, on: Boolean): Job? {
        val lh = lighthouse
        if (lh == null || !ready) {
            notReady()
            return null
        }
        jobs[mac]?.cancel()
        setLive(mac) { it.copy(pending = on) }
        return track(mac) {
            val target = if (on) Power.ON else store.offMode.first().byte
            try {
                val state = PowerState.of(lh.write(mac, target))
                setLive(mac) { Live(power = state, pending = null) }
                if (state == PowerState.Starting) pollUntilOn(mac)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                setLive(mac) { Live(power = PowerState.Unreachable, pending = null) }
                val name = state.value.stations.firstOrNull { it.station.mac == mac }?.station?.name ?: mac
                _messages.tryEmit("Couldn't turn $name ${if (on) "on" else "off"}: ${e.message ?: "unreachable"}")
            }
        }
    }

    /** Turns every managed station on or off at once. */
    fun setAll(on: Boolean) {
        if (!ready) {
            notReady()
            return
        }
        val managed = state.value.stations.filter { it.station.managed }
        if (managed.isEmpty()) return
        masterPending.value = on
        managed.forEach { setPower(it.station.mac, on) }
    }

    /** After waking up, a station reports startup values for a while; keep reading until it's on. */
    private suspend fun pollUntilOn(mac: String) {
        val lh = lighthouse ?: return
        for (i in 0 until 20) {
            delay(3_000)
            val byte = try {
                lh.read(mac)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                continue
            }
            val power = PowerState.of(byte)
            setLive(mac) { it.copy(power = power) }
            if (power != PowerState.Starting) return
        }
    }

    fun scan() {
        val lh = lighthouse
        if (scanning.value) return
        if (lh == null || !ready) {
            notReady()
            return
        }
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
            val lm = getApplication<Application>().getSystemService(LocationManager::class.java)
            if (lm != null && !LocationManagerCompat.isLocationEnabled(lm)) {
                _messages.tryEmit("Turn on Location to scan. Android 11 and lower need it to find Bluetooth devices.")
                return
            }
        }
        scanning.value = true
        viewModelScope.launch {
            val found = linkedMapOf<String, Found>()
            try {
                withTimeoutOrNull(SCAN_MS) {
                    lh.scan().collect { found[it.mac] = it }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _messages.tryEmit(e.message ?: "Scan failed")
            } finally {
                scanning.value = false
            }
            val added = store.addFound(found.values.map { Station(it.mac, it.name) })
            _messages.tryEmit(
                when {
                    found.isEmpty() -> "No base stations found. Check they're plugged in and nearby."
                    added.isEmpty() -> "No new base stations"
                    added.size == 1 -> "Added ${added[0].name}"
                    else -> "Added ${added.size} base stations"
                },
            )
            added.forEach { refresh(it.mac) }
        }
    }

    fun setOffMode(mode: OffMode) {
        viewModelScope.launch { store.setOffMode(mode) }
    }

    fun rename(mac: String, name: String) {
        val trimmed = name.trim()
        if (trimmed.isEmpty()) return
        viewModelScope.launch { store.update(mac) { it.copy(name = trimmed) } }
    }

    fun setManaged(mac: String, managed: Boolean) {
        viewModelScope.launch { store.update(mac) { it.copy(managed = managed) } }
    }

    fun remove(mac: String) {
        jobs.remove(mac)?.cancel()
        live.update { it - mac }
        viewModelScope.launch { store.remove(mac) }
    }

    private fun notReady() {
        _messages.tryEmit(
            when {
                lighthouse == null -> "This phone has no Bluetooth"
                !permitted.value -> "Allow Nearby devices access first"
                else -> "Turn on Bluetooth first"
            },
        )
    }

    private fun setLive(mac: String, change: (Live) -> Live) {
        live.update { it + (mac to change(it[mac] ?: Live())) }
    }

    /** Runs [block] as the station's current job, forgetting it when it ends. */
    private fun track(mac: String, block: suspend () -> Unit): Job {
        lateinit var job: Job
        job = viewModelScope.launch(start = CoroutineStart.LAZY) {
            try {
                block()
            } finally {
                if (jobs[mac] === job) jobs.remove(mac)
            }
        }
        jobs[mac] = job
        job.start()
        return job
    }

    private companion object {
        const val SCAN_MS = 6_000L
    }
}
