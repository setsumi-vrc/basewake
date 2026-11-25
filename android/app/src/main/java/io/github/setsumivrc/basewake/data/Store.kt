package io.github.setsumivrc.basewake.data

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import io.github.setsumivrc.basewake.ble.Power
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map
import org.json.JSONArray
import org.json.JSONObject

/** A remembered base station. Managed stations follow the master switch. */
data class Station(val mac: String, val name: String, val managed: Boolean = true)

/** What "off" means: fully off, or spun down and quicker to wake. */
enum class OffMode(val byte: Int, val label: String) {
    Sleep(Power.SLEEP, "Sleep"),
    Standby(Power.STANDBY, "Standby"),
}

private val Context.dataStore: DataStore<Preferences> by preferencesDataStore(name = "basewake")

/** Remembered stations and settings, kept in DataStore. */
class Store(context: Context) {
    private val store = context.applicationContext.dataStore

    val stations: Flow<List<Station>> = store.data.map { decode(it[STATIONS]) }

    val offMode: Flow<OffMode> = store.data.map { prefs ->
        OffMode.entries.firstOrNull { it.name == prefs[OFF_MODE] } ?: OffMode.Sleep
    }

    suspend fun setOffMode(mode: OffMode) {
        store.edit { it[OFF_MODE] = mode.name }
    }

    /** Adds stations not seen before. Returns the ones that were new. */
    suspend fun addFound(found: List<Station>): List<Station> {
        var added = emptyList<Station>()
        store.edit { prefs ->
            val list = decode(prefs[STATIONS])
            val known = list.map { it.mac }.toSet()
            added = found.filter { it.mac !in known }.distinctBy { it.mac }
            if (added.isNotEmpty()) prefs[STATIONS] = encode(list + added)
        }
        return added
    }

    suspend fun update(mac: String, change: (Station) -> Station) = edit { list ->
        list.map { if (it.mac == mac) change(it) else it }
    }

    suspend fun remove(mac: String) = edit { list -> list.filterNot { it.mac == mac } }

    private suspend fun edit(change: (List<Station>) -> List<Station>) {
        store.edit { prefs -> prefs[STATIONS] = encode(change(decode(prefs[STATIONS]))) }
    }

    private companion object {
        val STATIONS = stringPreferencesKey("stations")
        val OFF_MODE = stringPreferencesKey("off_mode")

        fun decode(json: String?): List<Station> {
            if (json.isNullOrEmpty()) return emptyList()
            return runCatching {
                val array = JSONArray(json)
                (0 until array.length()).map { i ->
                    val o = array.getJSONObject(i)
                    Station(o.getString("mac"), o.getString("name"), o.optBoolean("managed", true))
                }
            }.getOrDefault(emptyList())
        }

        fun encode(list: List<Station>): String = JSONArray().apply {
            list.forEach { s ->
                put(JSONObject().put("mac", s.mac).put("name", s.name).put("managed", s.managed))
            }
        }.toString()
    }
}
