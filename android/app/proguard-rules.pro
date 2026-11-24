-keep class io.github.tatarinovs.belka.core.BelkaNative { *; }
-keep class io.github.tatarinovs.belka.core.UrlOpener { *; }
-keep class io.github.tatarinovs.belka.BackupWorker { *; }
-keep class io.github.tatarinovs.belka.TransferService { *; }
-keepattributes *Annotation*,InnerClasses
-keep,includedescriptorclasses class io.github.tatarinovs.belka.core.**$$serializer { *; }
-keepclassmembers class io.github.tatarinovs.belka.core.** {
    *** Companion;
}

# WorkManager's Room database is created by reflection; full R8 optimization
# strips its constructor otherwise.
-keep class * extends androidx.room.RoomDatabase {
    <init>();
}
